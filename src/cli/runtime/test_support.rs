//! Bounded asynchronous test helpers for CLI runtime tests.

use std::future::Future;
use std::time::Duration;

pub(crate) const STEP_TIMEOUT: Duration = Duration::from_secs(1);
pub(crate) const TEST_WATCHDOG: Duration = Duration::from_secs(5);

pub(crate) struct TaskGuard<T> {
    handle: Option<tokio::task::JoinHandle<T>>,
}

impl<T> TaskGuard<T> {
    pub(crate) fn new(handle: tokio::task::JoinHandle<T>) -> Self {
        Self {
            handle: Some(handle),
        }
    }

    pub(crate) async fn join_raw(
        mut self,
        context: &str,
    ) -> Result<Result<T, tokio::task::JoinError>, String> {
        let mut handle = self.handle.take().expect("task guard handle");
        match tokio::time::timeout(STEP_TIMEOUT, &mut handle).await {
            Ok(result) => Ok(result),
            Err(_) => {
                handle.abort();
                tokio::time::timeout(STEP_TIMEOUT, &mut handle)
                    .await
                    .map_err(|_| format!("{context} cleanup timed out"))
                    .map(|_| ())?;
                Err(format!("{context} timed out"))
            }
        }
    }

    pub(crate) async fn join(self, context: &str) -> Result<T, String> {
        self.join_raw(context)
            .await?
            .map_err(|error| format!("{context} failed: {error}"))
    }

    pub(crate) fn abort(&self) {
        if let Some(handle) = &self.handle {
            handle.abort();
        }
    }
}

impl<T> Drop for TaskGuard<T> {
    fn drop(&mut self) {
        if let Some(handle) = &self.handle {
            handle.abort();
        }
    }
}

pub(crate) async fn bounded<F: Future>(future: F, context: &str) -> Result<F::Output, String> {
    tokio::time::timeout(STEP_TIMEOUT, future)
        .await
        .map_err(|_| format!("{context} timed out"))
}
