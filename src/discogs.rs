use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use std::fmt;

pub const BROKER_URL_ENV: &str = "REKLAWDBOX_DISCOGS_BROKER_URL";
pub const BROKER_TOKEN_ENV: &str = "REKLAWDBOX_DISCOGS_BROKER_TOKEN";

pub const DEFAULT_BROKER_URL: &str = "https://reklawdbox-discogs-broker.ryanvoitiskis.workers.dev";

pub(crate) const DEFAULT_BROKER_TOKEN: &str =
    "7d5596122d56ba256cb40ed9b1a6fb0724e45eb9b17399c687fc3cd649ce67ef";

#[derive(Debug, Clone)]
pub struct BrokerConfig {
    pub base_url: String,
    pub broker_token: Option<String>,
}

pub enum BrokerConfigStatus {
    Ok(BrokerConfig),
    InvalidUrl(String),
}

impl BrokerConfig {
    pub fn from_env() -> BrokerConfigStatus {
        let raw_base_url =
            std::env::var(BROKER_URL_ENV).unwrap_or_else(|_| DEFAULT_BROKER_URL.to_string());
        let base_url = match normalize_base_url(&raw_base_url) {
            Some(url) => url,
            None => return BrokerConfigStatus::InvalidUrl(raw_base_url),
        };
        let mut broker_token = env_var_trimmed_non_empty(BROKER_TOKEN_ENV);
        if broker_token.is_none() && base_url == DEFAULT_BROKER_URL {
            broker_token = Some(DEFAULT_BROKER_TOKEN.to_string());
        }
        BrokerConfigStatus::Ok(Self {
            base_url,
            broker_token,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingDeviceSession {
    pub device_id: String,
    pub pending_token: String,
    pub auth_url: String,
    pub poll_interval_seconds: i64,
    pub expires_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceSessionStatus {
    pub status: String,
    pub expires_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinalizedDeviceSession {
    pub session_token: String,
    pub expires_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthRemediation {
    pub message: String,
    pub auth_url: Option<String>,
    pub poll_interval_seconds: Option<i64>,
    pub expires_at: Option<i64>,
}

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
            Self::Message(msg) => write!(f, "{}", msg),
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

#[derive(Deserialize)]
struct DeviceSessionStartResponse {
    device_id: String,
    pending_token: String,
    auth_url: String,
    poll_interval_seconds: i64,
    expires_at: i64,
}

#[derive(Deserialize)]
struct DeviceSessionStatusResponse {
    status: String,
    expires_at: i64,
}

#[derive(Deserialize)]
struct DeviceSessionFinalizeResponse {
    session_token: String,
    expires_at: i64,
}

fn env_var_trimmed_non_empty(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

pub fn normalize_base_url(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let parsed = Url::parse(trimmed).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    parsed.host_str()?;
    let normalized = parsed.as_str().trim_end_matches('/').to_string();
    if normalized.is_empty() {
        return None;
    }
    Some(normalized)
}

pub fn pending_auth_remediation(pending: &PendingDeviceSession) -> AuthRemediation {
    AuthRemediation {
        message:
            "Discogs sign-in is still pending. Complete browser auth, then retry lookup_discogs."
                .to_string(),
        auth_url: Some(pending.auth_url.clone()),
        poll_interval_seconds: Some(pending.poll_interval_seconds),
        expires_at: Some(pending.expires_at),
    }
}

pub fn expired_session_remediation() -> AuthRemediation {
    AuthRemediation {
        message:
            "Discogs broker session is missing or expired. Re-run lookup_discogs to start auth."
                .to_string(),
        auth_url: None,
        poll_interval_seconds: None,
        expires_at: None,
    }
}

pub async fn device_session_start(
    client: &Client,
    cfg: &BrokerConfig,
) -> Result<PendingDeviceSession, String> {
    let mut request = client.post(format!("{}/v1/device/session/start", cfg.base_url));
    if let Some(token) = cfg.broker_token.as_deref() {
        request = request.header("x-reklawdbox-broker-token", token);
    }

    let response = request
        .send()
        .await
        .map_err(|e| format!("broker start request failed: {e}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if status == reqwest::StatusCode::UNAUTHORIZED && cfg.broker_token.is_none() {
            return Err(format!(
                "Broker returned 401 Unauthorized and no broker token is configured. \
                 Set {} to authenticate with your custom broker.",
                BROKER_TOKEN_ENV
            ));
        }
        return Err(format!("broker start HTTP {}: {}", status, body));
    }

    let payload: DeviceSessionStartResponse = response
        .json()
        .await
        .map_err(|e| format!("broker start JSON parse error: {e}"))?;

    Ok(PendingDeviceSession {
        device_id: payload.device_id,
        pending_token: payload.pending_token,
        auth_url: payload.auth_url,
        poll_interval_seconds: payload.poll_interval_seconds,
        expires_at: payload.expires_at,
    })
}

pub async fn device_session_status(
    client: &Client,
    cfg: &BrokerConfig,
    pending: &PendingDeviceSession,
) -> Result<DeviceSessionStatus, String> {
    let url = format!(
        "{}/v1/device/session/status?device_id={}&pending_token={}",
        cfg.base_url,
        urlencoding(&pending.device_id),
        urlencoding(&pending.pending_token)
    );
    let mut request = client.get(url);
    if let Some(token) = cfg.broker_token.as_deref() {
        request = request.header("x-reklawdbox-broker-token", token);
    }

    let response = request
        .send()
        .await
        .map_err(|e| format!("broker status request failed: {e}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("broker status HTTP {}: {}", status, body));
    }

    let payload: DeviceSessionStatusResponse = response
        .json()
        .await
        .map_err(|e| format!("broker status JSON parse error: {e}"))?;

    Ok(DeviceSessionStatus {
        status: payload.status,
        expires_at: payload.expires_at,
    })
}

pub async fn device_session_finalize(
    client: &Client,
    cfg: &BrokerConfig,
    pending: &PendingDeviceSession,
) -> Result<FinalizedDeviceSession, String> {
    let mut request = client.post(format!("{}/v1/device/session/finalize", cfg.base_url));
    if let Some(token) = cfg.broker_token.as_deref() {
        request = request.header("x-reklawdbox-broker-token", token);
    }
    request = request.json(&serde_json::json!({
        "device_id": pending.device_id,
        "pending_token": pending.pending_token,
    }));

    let response = request
        .send()
        .await
        .map_err(|e| format!("broker finalize request failed: {e}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("broker finalize HTTP {}: {}", status, body));
    }

    let payload: DeviceSessionFinalizeResponse = response
        .json()
        .await
        .map_err(|e| format!("broker finalize JSON parse error: {e}"))?;

    Ok(FinalizedDeviceSession {
        session_token: payload.session_token,
        expires_at: payload.expires_at,
    })
}

pub async fn lookup_via_broker(
    client: &Client,
    cfg: &BrokerConfig,
    session_token: &str,
    artist: &str,
    title: &str,
    album: Option<&str>,
) -> Result<Option<DiscogsResult>, LookupError> {
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
        let retry_after = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .map(String::from);
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

pub(crate) fn urlencoding(s: &str) -> String {
    use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
    const SET: &AsciiSet = &NON_ALPHANUMERIC
        .remove(b'-')
        .remove(b'_')
        .remove(b'.')
        .remove(b'~');
    utf8_percent_encode(s, SET).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn normalize_base_url_rejects_blank_or_malformed_urls() {
        assert_eq!(
            normalize_base_url("https://broker.example.com/"),
            Some("https://broker.example.com".to_string())
        );
        assert_eq!(normalize_base_url("   "), None);
        assert_eq!(normalize_base_url("///"), None);
        assert_eq!(normalize_base_url("https://"), None);
        assert_eq!(normalize_base_url("http:///"), None);
        assert_eq!(normalize_base_url("ftp://broker.example.com"), None);
    }
}
