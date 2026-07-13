//! Discogs broker configuration.

use reqwest::Url;

pub const BROKER_URL_ENV: &str = "REKLAWDBOX_DISCOGS_BROKER_URL";
pub const BROKER_TOKEN_ENV: &str = "REKLAWDBOX_DISCOGS_BROKER_TOKEN";

pub const DEFAULT_BROKER_URL: &str = "https://reklawdbox-discogs-broker.ryanvoitiskis.workers.dev";

// Public client token intentionally compiled for the maintained default broker;
// this is not a per-user secret or per-install credential.
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
        Self::from_raw(&raw_base_url, env_var_trimmed_non_empty(BROKER_TOKEN_ENV))
    }

    fn from_raw(raw_base_url: &str, configured_broker_token: Option<String>) -> BrokerConfigStatus {
        let base_url = match normalize_base_url(raw_base_url) {
            Some(url) => url,
            None => return BrokerConfigStatus::InvalidUrl(raw_base_url.to_string()),
        };
        let mut broker_token = configured_broker_token
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        if broker_token.is_none() && base_url == DEFAULT_BROKER_URL {
            broker_token = Some(DEFAULT_BROKER_TOKEN.to_string());
        }
        BrokerConfigStatus::Ok(Self {
            base_url,
            broker_token,
        })
    }
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn default_broker_url_gets_compiled_client_token() {
        let config = match BrokerConfig::from_raw(DEFAULT_BROKER_URL, None) {
            BrokerConfigStatus::Ok(config) => config,
            BrokerConfigStatus::InvalidUrl(url) => panic!("unexpected invalid URL: {url}"),
        };

        assert_eq!(config.base_url, DEFAULT_BROKER_URL);
        assert!(config.broker_token.is_some());
    }

    #[test]
    fn custom_broker_url_gets_no_token_unless_configured() {
        let config = match BrokerConfig::from_raw("https://broker.example.com", None) {
            BrokerConfigStatus::Ok(config) => config,
            BrokerConfigStatus::InvalidUrl(url) => panic!("unexpected invalid URL: {url}"),
        };

        assert_eq!(config.base_url, "https://broker.example.com");
        assert!(config.broker_token.is_none());
    }

    #[test]
    fn explicit_env_token_overrides_default_broker_token() {
        let config = match BrokerConfig::from_raw(
            DEFAULT_BROKER_URL,
            Some(" configured-client-token ".to_string()),
        ) {
            BrokerConfigStatus::Ok(config) => config,
            BrokerConfigStatus::InvalidUrl(url) => panic!("unexpected invalid URL: {url}"),
        };

        assert_eq!(
            config.broker_token.as_deref(),
            Some("configured-client-token")
        );
    }

    #[test]
    fn default_broker_url_with_trailing_slash_gets_compiled_client_token() {
        let config = match BrokerConfig::from_raw(
            "https://reklawdbox-discogs-broker.ryanvoitiskis.workers.dev/",
            None,
        ) {
            BrokerConfigStatus::Ok(config) => config,
            BrokerConfigStatus::InvalidUrl(url) => panic!("unexpected invalid URL: {url}"),
        };

        assert_eq!(config.base_url, DEFAULT_BROKER_URL);
        assert!(config.broker_token.is_some());
    }
}
