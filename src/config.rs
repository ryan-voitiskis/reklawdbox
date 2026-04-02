use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

const CONFIG_FILENAME: &str = "config.toml";

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub discogs: DiscogsConfig,
    /// Maps non-canonical genre strings (case-insensitive) to canonical genres.
    /// Supplements the built-in alias map; file entries take priority on conflict.
    #[serde(default)]
    pub genre_overrides: HashMap<String, String>,
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

/// SAFETY: must be called from main() before any threads are spawned.
pub unsafe fn apply_env(cfg: &Config) {
    if let Some(ref url) = cfg.discogs.broker.url
        && std::env::var_os(crate::discogs::BROKER_URL_ENV).is_none()
    {
        unsafe { std::env::set_var(crate::discogs::BROKER_URL_ENV, url) };
    }
    if let Some(ref token) = cfg.discogs.broker.token
        && std::env::var_os(crate::discogs::BROKER_TOKEN_ENV).is_none()
    {
        unsafe { std::env::set_var(crate::discogs::BROKER_TOKEN_ENV, token) };
    }
}

/// Sets owner-only permissions (0600) on the written file.
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
        assert!(cfg.genre_overrides.is_empty());
    }

    #[test]
    fn parses_genre_overrides() {
        let toml_str = r#"
[genre_overrides]
"Electronica" = "IDM"
"Bass" = "Breakbeat"
"#;
        let cfg: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.genre_overrides.len(), 2);
        assert_eq!(cfg.genre_overrides.get("Electronica").unwrap(), "IDM");
        assert_eq!(cfg.genre_overrides.get("Bass").unwrap(), "Breakbeat");
    }

    #[test]
    fn config_path_is_under_config_dir() {
        if let Some(path) = config_path() {
            assert!(path.ends_with("reklawdbox/config.toml"));
        }
    }
}
