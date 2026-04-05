use std::time::Duration;

use tokio::sync::Mutex as TokioMutex;
use tokio::time::Instant;

/// Shared rate limiter implementation. Each caller maintains its own
/// `TokioMutex<Option<Instant>>` for isolation; this function provides the
/// wait-and-update logic.
pub async fn wait(last: &TokioMutex<Option<Instant>>, env_var: &str, default_ms: u64) {
    let interval_ms: u64 = std::env::var(env_var)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default_ms);
    let interval = Duration::from_millis(interval_ms);

    let mut guard = last.lock().await;

    if let Some(prev) = *guard {
        let elapsed = prev.elapsed();
        if elapsed < interval {
            tokio::time::sleep(interval - elapsed).await;
        }
    }

    *guard = Some(Instant::now());
}

macro_rules! define_rate_limiter {
    ($env_var:literal, $default_ms:literal) => {
        static RATE_LIMITER: std::sync::OnceLock<tokio::sync::Mutex<Option<tokio::time::Instant>>> =
            std::sync::OnceLock::new();

        async fn wait_for_rate_limit() {
            let last = RATE_LIMITER.get_or_init(|| tokio::sync::Mutex::new(None));
            crate::rate_limit::wait(last, $env_var, $default_ms).await;
        }
    };
}
pub(crate) use define_rate_limiter;

pub fn extract_retry_after(headers: &reqwest::header::HeaderMap) -> Option<String> {
    headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
}
