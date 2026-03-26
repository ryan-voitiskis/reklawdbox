use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const CONFIG_FILENAME: &str = "config.toml";

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub discogs: DiscogsConfig,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct DiscogsConfig {
    #[serde(default)]
    pub broker: BrokerSection,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct BrokerSection {
    pub url: Option<String>,
    pub token: Option<String>,
}

/// Returns the platform config path for reklawdbox (`config.toml` inside
/// `dirs::config_dir()/reklawdbox`). On macOS this is
/// `~/Library/Application Support/reklawdbox/config.toml`.
pub fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("reklawdbox").join(CONFIG_FILENAME))
}

/// Load the config file, returning `Config::default()` if it doesn't exist.
/// Warns on stderr if the file exists but cannot be read or parsed.
pub fn load() -> Config {
    let Some(path) = config_path() else {
        return Config::default();
    };
    let contents = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Config::default(),
        Err(e) => {
            eprintln!("Warning: could not read {}: {e}", path.display());
            return Config::default();
        }
    };
    match toml::from_str(&contents) {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!(
                "Warning: failed to parse {}: {e}\nUsing default config.",
                path.display()
            );
            Config::default()
        }
    }
}

/// Populate environment variables from the config file.
/// Only sets variables that are not already present in the environment.
///
/// SAFETY: must be called from main() before any threads are spawned.
pub unsafe fn load_into_env() {
    let cfg = load();
    if let Some(url) = cfg.discogs.broker.url
        && std::env::var_os(crate::discogs::BROKER_URL_ENV).is_none()
    {
        unsafe { std::env::set_var(crate::discogs::BROKER_URL_ENV, url) };
    }
    if let Some(token) = cfg.discogs.broker.token
        && std::env::var_os(crate::discogs::BROKER_TOKEN_ENV).is_none()
    {
        unsafe { std::env::set_var(crate::discogs::BROKER_TOKEN_ENV, token) };
    }
}

/// Write a config to disk, creating the directory if needed.
/// Sets owner-only permissions (0600) on the config file.
pub fn save(cfg: &Config) -> Result<(), String> {
    let path = config_path().ok_or("Could not determine config directory")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create config directory: {e}"))?;
    }
    let contents =
        toml::to_string_pretty(cfg).map_err(|e| format!("Failed to serialize config: {e}"))?;
    std::fs::write(&path, contents).map_err(|e| format!("Failed to write config: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("Failed to set config file permissions: {e}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_round_trips_through_toml() {
        let cfg = Config::default();
        let s = toml::to_string_pretty(&cfg).unwrap();
        let parsed: Config = toml::from_str(&s).unwrap();
        assert!(parsed.discogs.broker.url.is_none());
        assert!(parsed.discogs.broker.token.is_none());
    }

    #[test]
    fn parses_populated_config() {
        let toml_str = r#"
[discogs.broker]
url = "https://example.com"
token = "abc123"
"#;
        let cfg: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(
            cfg.discogs.broker.url.as_deref(),
            Some("https://example.com")
        );
        assert_eq!(cfg.discogs.broker.token.as_deref(), Some("abc123"));
    }

    #[test]
    fn empty_toml_gives_defaults() {
        let cfg: Config = toml::from_str("").unwrap();
        assert!(cfg.discogs.broker.url.is_none());
        assert!(cfg.discogs.broker.token.is_none());
    }

    #[test]
    fn config_path_is_under_config_dir() {
        if let Some(path) = config_path() {
            assert!(path.ends_with("reklawdbox/config.toml"));
        }
    }
}
