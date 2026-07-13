//! Discogs device-session protocol and remediation payloads.

use reqwest::Client;
use serde::{Deserialize, Serialize};

use super::broker::{BROKER_TOKEN_ENV, BrokerConfig};
use crate::adapters::providers::http::urlencoding;

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

pub fn pending_auth_remediation(pending: &PendingDeviceSession) -> AuthRemediation {
    AuthRemediation {
        message: "Discogs auth required (not a lookup miss). Open the auth URL in the user's \
                  browser so they can authorize on Discogs, then call lookup_discogs again \u{2014} \
                  the next call picks up the new session automatically. Do not fall back to \
                  other enrichment sources for label/catalog data on commercial releases; \
                  Discogs is the authoritative source there."
            .to_string(),
        auth_url: Some(pending.auth_url.clone()),
        poll_interval_seconds: Some(pending.poll_interval_seconds),
        expires_at: Some(pending.expires_at),
    }
}

pub fn expired_session_remediation() -> AuthRemediation {
    AuthRemediation {
        message: "Discogs broker session is missing or expired (not a lookup miss). Call \
                  lookup_discogs again to start a new auth flow; it will return an auth URL to \
                  open."
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
                 Set {BROKER_TOKEN_ENV} to authenticate with your custom broker."
            ));
        }
        return Err(format!("broker start HTTP {status}: {body}"));
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
        return Err(format!("broker status HTTP {status}: {body}"));
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
        return Err(format!("broker finalize HTTP {status}: {body}"));
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
