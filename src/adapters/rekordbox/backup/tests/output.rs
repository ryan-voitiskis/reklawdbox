use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWriteExt, ReadBuf};

use super::super::output::{BoundedOutput, OutputReaderTask, read_bounded_output};

struct FailingReader;

impl AsyncRead for FailingReader {
    fn poll_read(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        _buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Poll::Ready(Err(io::Error::other("fixture read failed")))
    }
}

struct PanickingReader;

impl AsyncRead for PanickingReader {
    fn poll_read(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        _buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        panic!("fixture reader panic")
    }
}

async fn capture_bytes(count: usize) -> BoundedOutput {
    let (mut writer, reader) = tokio::io::duplex(256);
    let writer_task = tokio::spawn(async move {
        writer
            .write_all(&vec![b'x'; count])
            .await
            .expect("fixture output should write");
        writer
            .shutdown()
            .await
            .expect("fixture output should close");
    });
    let output = read_bounded_output(reader)
        .await
        .expect("fixture output should read");
    writer_task.await.expect("fixture writer should join");
    output
}

#[tokio::test]
async fn output_reader_cleanup_aborts_persistent_escaped_pipe_holders() {
    let (stdout_reader, mut stdout_holder) = tokio::io::duplex(64);
    let (stderr_reader, mut stderr_holder) = tokio::io::duplex(64);
    let activity = Arc::new(AtomicUsize::new(0));
    let mut stdout_task = OutputReaderTask::spawn("stdout", stdout_reader, Arc::clone(&activity));
    let mut stderr_task = OutputReaderTask::spawn("stderr", stderr_reader, Arc::clone(&activity));

    tokio::time::timeout(Duration::from_secs(1), async {
        while activity.load(Ordering::Acquire) != 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("both output readers should start");

    stdout_task.abort_if_running();
    stderr_task.abort_if_running();
    let (stdout, stderr) = tokio::time::timeout(Duration::from_secs(1), async {
        tokio::join!(
            stdout_task.finish_after_abort(),
            stderr_task.finish_after_abort()
        )
    })
    .await
    .expect("aborted output readers should finish");
    assert!(stdout.is_none());
    assert!(stderr.is_none());
    assert_eq!(activity.load(Ordering::Acquire), 0);
    assert!(stdout_holder.write_all(b"still open").await.is_err());
    assert!(stderr_holder.write_all(b"still open").await.is_err());
}

#[tokio::test]
async fn bounded_output_preserves_exact_limit_and_suffix_contract() {
    let below = capture_bytes(8 * 1024 - 1).await;
    assert_eq!(below.bytes.len(), 8 * 1024 - 1);
    assert!(!below.truncated);
    assert!(!below.render().ends_with(" …[truncated]"));

    let at_limit = capture_bytes(8 * 1024).await;
    assert_eq!(at_limit.bytes.len(), 8 * 1024);
    assert!(!at_limit.truncated);
    assert!(!at_limit.render().ends_with(" …[truncated]"));

    let above = capture_bytes(8 * 1024 + 1).await;
    assert_eq!(above.bytes.len(), 8 * 1024);
    assert!(above.truncated);
    assert!(above.render().ends_with(" …[truncated]"));
}

#[tokio::test]
async fn output_reader_read_failure_preserves_label_and_releases_activity() {
    let activity = Arc::new(AtomicUsize::new(0));
    let mut task = OutputReaderTask::spawn("stdout", FailingReader, Arc::clone(&activity));
    let error = match task.finish().await {
        Ok(_) => panic!("fixture reader should fail"),
        Err(error) => error,
    };
    assert_eq!(
        error.to_string(),
        "backup stdout capture failed: fixture read failed"
    );
    assert_eq!(activity.load(Ordering::Acquire), 0);
}

#[tokio::test]
async fn output_reader_panic_preserves_task_failure_category_and_releases_activity() {
    let activity = Arc::new(AtomicUsize::new(0));
    let mut task = OutputReaderTask::spawn("stderr", PanickingReader, Arc::clone(&activity));
    let error = match task.finish().await {
        Ok(_) => panic!("panicking reader task should fail"),
        Err(error) => error,
    };
    let message = error.to_string();
    assert!(
        message.starts_with("backup stderr capture task failed: task "),
        "unexpected reader panic message: {message}"
    );
    assert!(message.contains("panicked with message \"fixture reader panic\""));
    assert_eq!(activity.load(Ordering::Acquire), 0);
}
