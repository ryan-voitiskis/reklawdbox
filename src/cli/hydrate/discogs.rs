//! Discogs authentication, browser handoff, and CLI retry policy.

use std::time::Duration;

use indicatif::{ProgressBar, ProgressStyle};

use crate::adapters::providers::discogs;
use crate::adapters::state as store;

pub(super) async fn ensure_auth(
    client: &reqwest::Client,
    store_path: &str,
) -> Result<(discogs::BrokerConfig, String), Box<dyn std::error::Error>> {
    match discogs::BrokerConfig::from_env() {
        discogs::BrokerConfigStatus::Ok(cfg) => {
            let store_conn = store::open(store_path)?;
            if let Some(session) = store::get_broker_discogs_session(&store_conn, &cfg.base_url)? {
                let now = chrono::Utc::now().timestamp();
                if session.expires_at > now {
                    println!("Discogs: using existing broker session");
                    if session.expires_at - now < 3600 {
                        println!("  Warning: session expires in <1 hour");
                    }
                    return Ok((cfg, session.session_token));
                }
                // Expired — clear and re-auth
                store::clear_broker_discogs_session(&store_conn, &cfg.base_url)?;
            }
            drop(store_conn);

            println!("Discogs: starting broker authentication...");
            let pending = discogs::device_session_start(client, &cfg)
                .await
                .map_err(|e| format!("Failed to start Discogs auth: {e}"))?;

            println!("Please authorize at: {}", pending.auth_url);
            let _ = std::process::Command::new("open")
                .arg(&pending.auth_url)
                .spawn();

            let spinner = ProgressBar::new_spinner();
            spinner.set_style(
                ProgressStyle::with_template("{spinner:.green} Waiting for authorization...")
                    .unwrap(),
            );
            spinner.enable_steady_tick(Duration::from_millis(200));

            let poll_interval = Duration::from_secs(pending.poll_interval_seconds.max(2) as u64);

            loop {
                tokio::time::sleep(poll_interval).await;

                let now = chrono::Utc::now().timestamp();
                if now >= pending.expires_at {
                    spinner.finish_and_clear();
                    return Err("Discogs device auth session expired. Please retry.".into());
                }

                let status = discogs::device_session_status(client, &cfg, &pending)
                    .await
                    .map_err(|e| format!("Auth poll failed: {e}"))?;

                match status.status.as_str() {
                    "authorized" | "finalized" => {
                        let finalized = discogs::device_session_finalize(client, &cfg, &pending)
                            .await
                            .map_err(|e| format!("Auth finalize failed: {e}"))?;

                        let store_conn = store::open(store_path)?;
                        store::set_broker_discogs_session(
                            &store_conn,
                            &cfg.base_url,
                            &finalized.session_token,
                            finalized.expires_at,
                        )?;

                        spinner.finish_and_clear();
                        println!("Discogs: authenticated successfully");
                        return Ok((cfg, finalized.session_token));
                    }
                    "pending" => continue,
                    other => {
                        spinner.finish_and_clear();
                        return Err(format!("Unexpected auth status: {other}").into());
                    }
                }
            }
        }
        discogs::BrokerConfigStatus::InvalidUrl(url) => {
            Err(format!("Invalid Discogs broker URL: {url}").into())
        }
    }
}

pub(super) struct DiscogsHttpRetry<'a> {
    pub(super) status: u16,
    pub(super) wait_seconds: u64,
    pub(super) diagnostic_body: &'a str,
}

pub(super) fn http_retry_metadata(
    error: &discogs::LookupError,
    attempt: u32,
) -> Option<DiscogsHttpRetry<'_>> {
    let status = error.http_status()?;
    let wait_seconds = match status {
        429 => error
            .retry_after()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(5)
            .min(120),
        500..=599 => 5 * 2u64.pow(attempt),
        _ => return None,
    };
    Some(DiscogsHttpRetry {
        status,
        wait_seconds,
        diagnostic_body: error.diagnostic_body().unwrap_or_default(),
    })
}

async fn lookup_with_retry_using<Lookup, LookupFuture, Sleep, SleepFuture>(
    mut lookup: Lookup,
    mut sleep: Sleep,
) -> Result<Option<discogs::DiscogsResult>, discogs::LookupError>
where
    Lookup: FnMut() -> LookupFuture,
    LookupFuture:
        std::future::Future<Output = Result<Option<discogs::DiscogsResult>, discogs::LookupError>>,
    Sleep: FnMut(Duration) -> SleepFuture,
    SleepFuture: std::future::Future<Output = ()>,
{
    const MAX_ATTEMPTS: u32 = 4;

    for attempt in 0..MAX_ATTEMPTS {
        match lookup().await {
            Ok(result) => return Ok(result),
            Err(error) => {
                // Defence-in-depth: the broker handles Discogs 429s internally,
                // but platform-level rate limits (Cloudflare) or custom brokers
                // may 429.
                let backoff = if let Some(retry) = http_retry_metadata(&error, attempt) {
                    if retry.status == 429 {
                        tracing::warn!(
                            status = 429,
                            attempt,
                            wait = retry.wait_seconds,
                            "Discogs broker 429: {}",
                            retry.diagnostic_body
                        );
                    } else {
                        tracing::warn!(
                            status = retry.status,
                            attempt,
                            wait = retry.wait_seconds,
                            "Discogs broker {}: {}",
                            retry.status,
                            retry.diagnostic_body
                        );
                    }
                    Some(retry.wait_seconds)
                } else {
                    match &error {
                        discogs::LookupError::Message(message) => {
                            let wait = 5 * 2u64.pow(attempt);
                            tracing::warn!(
                                attempt,
                                wait,
                                "Discogs broker transport error: {message}"
                            );
                            Some(wait)
                        }
                        _ => None,
                    }
                };

                match backoff {
                    Some(seconds) if attempt < MAX_ATTEMPTS - 1 => {
                        sleep(Duration::from_secs(seconds)).await;
                    }
                    _ => return Err(error),
                }
            }
        }
    }

    unreachable!("loop always exits via return")
}

pub(super) async fn lookup_with_retry(
    client: &reqwest::Client,
    cfg: &discogs::BrokerConfig,
    token: &str,
    artist: &str,
    title: &str,
    album: Option<&str>,
) -> Result<Option<discogs::DiscogsResult>, discogs::LookupError> {
    lookup_with_retry_using(
        || discogs::lookup_via_broker(client, cfg, token, artist, title, album),
        tokio::time::sleep,
    )
    .await
}

#[cfg(test)]
pub(super) async fn lookup_with_retry_for_test<Lookup, LookupFuture, Sleep, SleepFuture>(
    lookup: Lookup,
    sleep: Sleep,
) -> Result<Option<discogs::DiscogsResult>, discogs::LookupError>
where
    Lookup: FnMut() -> LookupFuture,
    LookupFuture:
        std::future::Future<Output = Result<Option<discogs::DiscogsResult>, discogs::LookupError>>,
    Sleep: FnMut(Duration) -> SleepFuture,
    SleepFuture: std::future::Future<Output = ()>,
{
    lookup_with_retry_using(lookup, sleep).await
}
