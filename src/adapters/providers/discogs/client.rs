//! Discogs broker lookup client and response parsing.

use std::fmt;

use reqwest::Client;
use serde::{Deserialize, Serialize};

use super::auth::{AuthRemediation, expired_session_remediation};
use super::broker::BrokerConfig;
use super::{RATE_LIMITER, wait_for_rate_limit};
use crate::adapters::providers::rate_limit;

#[derive(Debug, Clone)]
pub enum LookupError {
    AuthRequired(AuthRemediation),
    Http {
        status: u16,
        retry_after: Option<String>,
        body: String,
    },
    Message(String),
}

impl LookupError {
    pub fn auth_remediation(&self) -> Option<&AuthRemediation> {
        match self {
            Self::AuthRequired(remediation) => Some(remediation),
            Self::Http { .. } | Self::Message(_) => None,
        }
    }

    pub fn message(msg: impl Into<String>) -> Self {
        Self::Message(msg.into())
    }
}

impl fmt::Display for LookupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AuthRequired(remediation) => {
                if let Some(auth_url) = remediation.auth_url.as_deref() {
                    write!(f, "{} Open: {}", remediation.message, auth_url)
                } else {
                    write!(f, "{}", remediation.message)
                }
            }
            Self::Http { status, body, .. } => write!(f, "broker proxy HTTP {status}: {body}"),
            Self::Message(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for LookupError {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscogsResult {
    pub title: String,
    pub year: String,
    pub label: String,
    pub genres: Vec<String>,
    pub styles: Vec<String>,
    pub url: String,
    #[serde(default)]
    pub cover_image: String,
    pub fuzzy_match: bool,
}

pub async fn lookup_via_broker(
    client: &Client,
    cfg: &BrokerConfig,
    session_token: &str,
    artist: &str,
    title: &str,
    album: Option<&str>,
) -> Result<Option<DiscogsResult>, LookupError> {
    wait_for_rate_limit().await;

    let payload = serde_json::json!({
        "artist": artist,
        "title": title,
        "album": album,
    });

    let response = client
        .post(format!("{}/v1/discogs/proxy/search", cfg.base_url))
        .bearer_auth(session_token)
        .json(&payload)
        .send()
        .await
        .map_err(|e| LookupError::message(format!("broker proxy request failed: {e}")))?;

    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err(LookupError::AuthRequired(expired_session_remediation()));
    }

    if !response.status().is_success() {
        let status = response.status().as_u16();
        let retry_after = rate_limit::extract_retry_after(response.headers());
        let body = response.text().await.unwrap_or_default();
        return Err(LookupError::Http {
            status,
            retry_after,
            body,
        });
    }

    let json: serde_json::Value = response
        .json()
        .await
        .map_err(|e| LookupError::message(format!("broker proxy JSON parse error: {e}")))?;

    parse_broker_lookup_payload(json).map_err(LookupError::message)
}

pub(crate) fn parse_broker_lookup_payload(
    payload: serde_json::Value,
) -> Result<Option<DiscogsResult>, String> {
    if payload.is_null() {
        return Ok(None);
    }

    if let Some(result_value) = payload.get("result") {
        if result_value.is_null() {
            return Ok(None);
        }
        return serde_json::from_value::<DiscogsResult>(result_value.clone())
            .map(Some)
            .map_err(|e| format!("invalid broker result payload: {e}"));
    }

    serde_json::from_value::<DiscogsResult>(payload)
        .map(Some)
        .map_err(|e| format!("invalid broker payload: {e}"))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::time::Instant;

    use super::*;

    #[tokio::test]
    async fn rate_limiter_enforces_minimum_spacing() {
        // SAFETY: test runs sequentially (no other threads reading this env var).
        unsafe { std::env::set_var("REKLAWDBOX_DISCOGS_MIN_INTERVAL_MS", "50") };

        let n = 4;
        let start = Instant::now();

        for _ in 0..n {
            wait_for_rate_limit().await;
        }

        let elapsed = start.elapsed();
        let min_expected = Duration::from_millis(50 * (n - 1));
        assert!(
            elapsed >= min_expected,
            "expected >= {min_expected:?}, got {elapsed:?}"
        );

        *RATE_LIMITER.get().unwrap().lock().await = None;

        // SAFETY: cleaning up test-only env var.
        unsafe { std::env::remove_var("REKLAWDBOX_DISCOGS_MIN_INTERVAL_MS") };
    }

    #[test]
    fn lookup_error_message_displays_and_has_no_auth_remediation() {
        let err = LookupError::message("broker proxy request failed: connection reset");
        assert_eq!(
            err.to_string(),
            "broker proxy request failed: connection reset"
        );
        assert!(err.auth_remediation().is_none());
    }

    #[test]
    fn lookup_error_http_5xx_displays_status_and_body() {
        let err = LookupError::Http {
            status: 502,
            retry_after: None,
            body: "bad gateway".to_string(),
        };
        assert!(err.to_string().contains("502"));
        assert!(err.to_string().contains("bad gateway"));
        assert!(err.auth_remediation().is_none());
    }

    #[test]
    fn parse_broker_payload_with_wrapped_result() {
        let payload = serde_json::json!({
            "result": {
                "title": "Artist - Title",
                "year": "2024",
                "label": "Label",
                "genres": ["Electronic"],
                "styles": ["Deep House"],
                "url": "https://www.discogs.com/release/1",
                "fuzzy_match": false
            },
            "match_quality": "exact",
            "cache_hit": false
        });

        let parsed = parse_broker_lookup_payload(payload)
            .expect("payload should parse")
            .expect("result should exist");
        assert_eq!(parsed.title, "Artist - Title");
        assert_eq!(parsed.label, "Label");
        assert_eq!(parsed.styles, vec!["Deep House"]);
    }

    #[test]
    fn parse_broker_payload_with_null_result() {
        let payload = serde_json::json!({
            "result": null,
            "match_quality": "none",
            "cache_hit": true
        });
        let parsed = parse_broker_lookup_payload(payload).expect("payload should parse");
        assert!(parsed.is_none());
    }

    #[test]
    fn parse_broker_payload_direct_result_object() {
        let payload = serde_json::json!({
            "title": "Artist - Title",
            "year": "2024",
            "label": "Label",
            "genres": ["Electronic"],
            "styles": ["Techno"],
            "url": "https://www.discogs.com/release/2",
            "fuzzy_match": true
        });
        let parsed = parse_broker_lookup_payload(payload)
            .expect("payload should parse")
            .expect("result should exist");
        assert!(parsed.fuzzy_match);
        assert_eq!(parsed.styles, vec!["Techno"]);
    }
}
