use rand::Rng;
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::OnceLock;

pub const BROKER_URL_ENV: &str = "REKLAWDBOX_DISCOGS_BROKER_URL";
pub const BROKER_TOKEN_ENV: &str = "REKLAWDBOX_DISCOGS_BROKER_TOKEN";

pub const LEGACY_KEY_ENV: &str = "REKLAWDBOX_DISCOGS_KEY";
pub const LEGACY_SECRET_ENV: &str = "REKLAWDBOX_DISCOGS_SECRET";
pub const LEGACY_TOKEN_ENV: &str = "REKLAWDBOX_DISCOGS_TOKEN";
pub const LEGACY_TOKEN_SECRET_ENV: &str = "REKLAWDBOX_DISCOGS_TOKEN_SECRET";

pub const DISCOGS_API_BASE_URL_ENV: &str = "REKLAWDBOX_DISCOGS_API_BASE_URL";

#[derive(Debug, Clone)]
pub struct BrokerConfig {
    pub base_url: String,
    pub broker_token: Option<String>,
}

impl BrokerConfig {
    pub fn from_env() -> Option<Self> {
        let raw_base_url = std::env::var(BROKER_URL_ENV).ok()?;
        let base_url = normalize_base_url(&raw_base_url)?;
        let broker_token = env_var_trimmed_non_empty(BROKER_TOKEN_ENV);
        Some(Self {
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
    Message(String),
}

impl LookupError {
    pub fn auth_remediation(&self) -> Option<&AuthRemediation> {
        match self {
            Self::AuthRequired(remediation) => Some(remediation),
            Self::Message(_) => None,
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
struct SearchResponse {
    results: Option<Vec<SearchResult>>,
}

#[derive(Deserialize)]
struct SearchResult {
    title: Option<String>,
    year: Option<String>,
    label: Option<Vec<String>>,
    genre: Option<Vec<String>>,
    style: Option<Vec<String>>,
    uri: Option<String>,
    cover_image: Option<String>,
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

/// Normalize a string for matching: lowercase, strip non-alphanumeric.
pub fn normalize(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == ' ')
        .collect::<String>()
        .trim()
        .to_string()
}

fn env_var_trimmed_non_empty(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn normalize_base_url(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let parsed = Url::parse(trimmed).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    if parsed.host_str().is_none() {
        return None;
    }
    let normalized = parsed.as_str().trim_end_matches('/').to_string();
    if normalized.is_empty() {
        return None;
    }
    Some(normalized)
}

fn legacy_values_configured(
    key: Option<&str>,
    secret: Option<&str>,
    token: Option<&str>,
    token_secret: Option<&str>,
) -> bool {
    [key, secret, token, token_secret]
        .iter()
        .all(|v| v.is_some_and(|s| !s.trim().is_empty()))
}

pub fn legacy_credentials_configured() -> bool {
    let key = std::env::var(LEGACY_KEY_ENV).ok();
    let secret = std::env::var(LEGACY_SECRET_ENV).ok();
    let token = std::env::var(LEGACY_TOKEN_ENV).ok();
    let token_secret = std::env::var(LEGACY_TOKEN_SECRET_ENV).ok();
    legacy_values_configured(
        key.as_deref(),
        secret.as_deref(),
        token.as_deref(),
        token_secret.as_deref(),
    )
}

pub fn missing_auth_remediation() -> AuthRemediation {
    AuthRemediation {
        message: format!(
            "Discogs auth is not configured. Set {} to use broker auth (recommended), then retry.
Legacy {} credentials are deprecated from the default path.",
            BROKER_URL_ENV, LEGACY_KEY_ENV
        ),
        auth_url: None,
        poll_interval_seconds: None,
        expires_at: None,
    }
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
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(LookupError::message(format!(
            "broker proxy HTTP {}: {}",
            status, body
        )));
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

struct Credentials {
    consumer_key: String,
    signature: String,
    token: String,
}

static CREDENTIALS: OnceLock<Result<Credentials, String>> = OnceLock::new();

fn get_credentials() -> Result<&'static Credentials, String> {
    let result = CREDENTIALS.get_or_init(|| {
        let key = env_var_trimmed_non_empty(LEGACY_KEY_ENV)
            .ok_or_else(|| format!("{} not set or empty", LEGACY_KEY_ENV))?;
        let secret = env_var_trimmed_non_empty(LEGACY_SECRET_ENV)
            .ok_or_else(|| format!("{} not set or empty", LEGACY_SECRET_ENV))?;
        let token = env_var_trimmed_non_empty(LEGACY_TOKEN_ENV)
            .ok_or_else(|| format!("{} not set or empty", LEGACY_TOKEN_ENV))?;
        let token_secret = env_var_trimmed_non_empty(LEGACY_TOKEN_SECRET_ENV)
            .ok_or_else(|| format!("{} not set or empty", LEGACY_TOKEN_SECRET_ENV))?;
        let signature = format!("{secret}&{token_secret}");
        Ok(Credentials {
            consumer_key: key,
            signature,
            token,
        })
    });
    result.as_ref().map_err(|e| e.clone())
}

fn nonce() -> String {
    let mut rng = rand::rng();
    let bytes: [u8; 16] = rng.random();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn build_legacy_search_url(base_url: &str, query_params: &str) -> String {
    format!(
        "{}/database/search?{query_params}",
        base_url.trim_end_matches('/')
    )
}

fn build_legacy_oauth_authorization_header(
    creds: &Credentials,
    oauth_nonce: &str,
    timestamp: u64,
) -> String {
    format!(
        "OAuth oauth_consumer_key=\"{consumer_key}\", oauth_nonce=\"{nonce}\", oauth_signature=\"{signature}\", oauth_signature_method=\"PLAINTEXT\", oauth_timestamp=\"{timestamp}\", oauth_token=\"{token}\", oauth_version=\"1.0\"",
        consumer_key = urlencoding(&creds.consumer_key),
        nonce = urlencoding(oauth_nonce),
        signature = urlencoding(&creds.signature),
        token = urlencoding(&creds.token),
    )
}

pub async fn lookup_with_legacy_credentials(
    client: &Client,
    artist: &str,
    title: &str,
    album: Option<&str>,
) -> Result<Option<DiscogsResult>, String> {
    lookup_inner_legacy(client, artist, title, album, false).await
}

async fn lookup_inner_legacy(
    client: &Client,
    artist: &str,
    title: &str,
    album: Option<&str>,
    is_retry: bool,
) -> Result<Option<DiscogsResult>, String> {
    // Rate limit
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;

    let creds = get_credentials()?;

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut query_params = format!(
        "artist={artist}&track={track}&type=release&per_page=15",
        artist = urlencoding(artist),
        track = urlencoding(title),
    );
    if let Some(album) = album.filter(|a| !a.is_empty()) {
        query_params.push_str(&format!("&release_title={}", urlencoding(album)));
    }

    let base_url = std::env::var(DISCOGS_API_BASE_URL_ENV)
        .ok()
        .and_then(|raw| normalize_base_url(&raw))
        .unwrap_or_else(|| "https://api.discogs.com".to_string());

    let oauth_nonce = nonce();
    let url = build_legacy_search_url(&base_url, &query_params);
    let auth_header = build_legacy_oauth_authorization_header(creds, &oauth_nonce, timestamp);

    let resp = client
        .get(&url)
        .header(reqwest::header::AUTHORIZATION, auth_header)
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;

    if resp.status() == 429 {
        if is_retry {
            return Err("rate limited after retry".into());
        }
        eprintln!("[reklawdbox] Discogs rate limited, waiting 30s...");
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        return Box::pin(lookup_inner_legacy(client, artist, title, album, true)).await;
    }

    if !resp.status().is_success() {
        return Err(format!("Discogs HTTP {}", resp.status()));
    }

    let data: SearchResponse = resp
        .json()
        .await
        .map_err(|e| format!("JSON parse error: {e}"))?;

    let results = match data.results {
        Some(r) if !r.is_empty() => r,
        _ => return Ok(None),
    };

    // Find best match by artist name in result title.
    for r in &results {
        let result_title = r.title.as_deref().unwrap_or("");
        if result_title_matches_artist(result_title, artist) {
            return Ok(Some(to_result(r, false)));
        }
    }

    // Fallback to first result
    Ok(Some(to_result(&results[0], true)))
}

fn to_result(r: &SearchResult, fuzzy: bool) -> DiscogsResult {
    let url = r
        .uri
        .as_deref()
        .map(|uri| format!("https://www.discogs.com{uri}"))
        .unwrap_or_default();
    DiscogsResult {
        title: r.title.clone().unwrap_or_default(),
        year: r.year.clone().unwrap_or_default(),
        label: r
            .label
            .as_ref()
            .and_then(|v| v.first().cloned())
            .unwrap_or_default(),
        genres: r.genre.clone().unwrap_or_default(),
        styles: r.style.clone().unwrap_or_default(),
        url,
        cover_image: r.cover_image.clone().unwrap_or_default(),
        fuzzy_match: fuzzy,
    }
}

fn result_title_matches_artist(result_title: &str, artist: &str) -> bool {
    let norm_artist = normalize(artist);
    if norm_artist.len() < 3 {
        return true;
    }
    normalize(result_title).contains(&norm_artist)
}

fn urlencoding(s: &str) -> String {
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
    fn result_title_match_handles_punctuation() {
        assert!(result_title_matches_artist(
            "A$AP Rocky - Praise The Lord",
            "A$AP Rocky"
        ));
    }

    #[test]
    fn result_title_match_allows_short_artist_names() {
        assert!(result_title_matches_artist("Random Result", "DJ"));
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

    #[test]
    fn legacy_values_configured_requires_non_empty_values() {
        assert!(legacy_values_configured(
            Some("key"),
            Some("secret"),
            Some("token"),
            Some("token-secret")
        ));
        assert!(!legacy_values_configured(
            Some("key"),
            Some("  "),
            Some("token"),
            Some("token-secret")
        ));
        assert!(!legacy_values_configured(
            Some("key"),
            None,
            Some("token"),
            Some("token-secret")
        ));
    }

    #[test]
    fn legacy_request_keeps_oauth_secrets_out_of_url() {
        let creds = Credentials {
            consumer_key: "consumer key".to_string(),
            signature: "secret/part&token?secret".to_string(),
            token: "token value".to_string(),
        };
        let query = "artist=Artist&track=Title&type=release&per_page=15";
        let url = build_legacy_search_url("https://api.discogs.com", query);
        let auth =
            build_legacy_oauth_authorization_header(&creds, "nonce value", 1_700_000_000_u64);

        assert_eq!(
            url,
            "https://api.discogs.com/database/search?artist=Artist&track=Title&type=release&per_page=15"
        );
        assert!(!url.contains("oauth_"));
        assert!(auth.starts_with("OAuth "));
        assert!(auth.contains("oauth_consumer_key=\"consumer%20key\""));
        assert!(auth.contains("oauth_nonce=\"nonce%20value\""));
        assert!(auth.contains("oauth_signature=\"secret%2Fpart%26token%3Fsecret\""));
        assert!(auth.contains("oauth_token=\"token%20value\""));
    }
}
