#[cfg(target_os = "macos")]
mod imp {
    use security_framework::passwords;

    const SERVICE: &str = "com.reklawdbox.broker-session";

    const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;

    pub fn set_session_token(broker_url: &str, token: &str) -> Result<(), String> {
        passwords::set_generic_password(SERVICE, broker_url, token.as_bytes())
            .map_err(|e| format!("Keychain write failed: {e}"))
    }

    pub fn get_session_token(broker_url: &str) -> Result<Option<String>, String> {
        match passwords::get_generic_password(SERVICE, broker_url) {
            Ok(bytes) => String::from_utf8(bytes)
                .map(Some)
                .map_err(|e| format!("Keychain value is not UTF-8: {e}")),
            Err(e) if e.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(None),
            Err(e) => Err(format!("Keychain read failed: {e}")),
        }
    }

    pub fn delete_session_token(broker_url: &str) -> Result<(), String> {
        match passwords::delete_generic_password(SERVICE, broker_url) {
            Ok(()) => Ok(()),
            Err(e) if e.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(()),
            Err(e) => Err(format!("Keychain delete failed: {e}")),
        }
    }
}

/// Without a system keychain (Linux, WSL, containers) broker session tokens
/// live in an owner-only JSON file: `broker-sessions.json` inside
/// `$REKLAWDBOX_SECRETS_DIR`, or `dirs::config_dir()/reklawdbox` by default
/// (`~/.config/reklawdbox` on Linux). In a cluster, point
/// `REKLAWDBOX_SECRETS_DIR` at a persistent volume.
#[cfg(not(target_os = "macos"))]
mod imp {
    use std::collections::BTreeMap;
    use std::io::Write as _;
    use std::path::{Path, PathBuf};

    pub(super) const SECRETS_DIR_ENV: &str = "REKLAWDBOX_SECRETS_DIR";
    const FILE_NAME: &str = "broker-sessions.json";

    fn store_path() -> Result<PathBuf, String> {
        let dir = match std::env::var_os(SECRETS_DIR_ENV) {
            Some(dir) => PathBuf::from(dir),
            None => dirs::config_dir()
                .ok_or("Could not determine config directory for broker sessions")?
                .join("reklawdbox"),
        };
        Ok(dir.join(FILE_NAME))
    }

    pub(super) fn read_all(path: &Path) -> Result<BTreeMap<String, String>, String> {
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| format!("Broker session store {} is corrupt: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(e) => Err(format!(
                "Failed to read broker session store {}: {e}",
                path.display()
            )),
        }
    }

    pub(super) fn write_all(path: &Path, tokens: &BTreeMap<String, String>) -> Result<(), String> {
        let parent = path
            .parent()
            .ok_or("Broker session store path has no parent directory")?;
        // Only tighten a directory we create; an existing one (e.g. a mounted
        // volume set via REKLAWDBOX_SECRETS_DIR) keeps its owner's permissions.
        let created = !parent.exists();
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create {}: {e}", parent.display()))?;
        #[cfg(unix)]
        if created {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
                .map_err(|e| format!("Failed to restrict {}: {e}", parent.display()))?;
        }
        #[cfg(not(unix))]
        let _ = created;
        let mut temp = tempfile::Builder::new()
            .prefix(".broker-sessions-")
            .tempfile_in(parent)
            .map_err(|e| format!("Failed to create temporary session store: {e}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            temp.as_file()
                .set_permissions(std::fs::Permissions::from_mode(0o600))
                .map_err(|e| format!("Failed to set session store permissions: {e}"))?;
        }
        let bytes = serde_json::to_vec_pretty(tokens)
            .map_err(|e| format!("Failed to serialize session store: {e}"))?;
        temp.write_all(&bytes)
            .map_err(|e| format!("Failed to write session store: {e}"))?;
        temp.as_file_mut()
            .sync_all()
            .map_err(|e| format!("Failed to sync session store: {e}"))?;
        temp.persist(path)
            .map_err(|e| format!("Failed to replace session store: {}", e.error))?;
        Ok(())
    }

    pub(super) fn set_in(path: &Path, broker_url: &str, token: &str) -> Result<(), String> {
        let mut tokens = read_all(path)?;
        tokens.insert(broker_url.to_string(), token.to_string());
        write_all(path, &tokens)
    }

    pub(super) fn get_in(path: &Path, broker_url: &str) -> Result<Option<String>, String> {
        Ok(read_all(path)?.remove(broker_url))
    }

    pub(super) fn delete_in(path: &Path, broker_url: &str) -> Result<(), String> {
        let mut tokens = read_all(path)?;
        if tokens.remove(broker_url).is_some() {
            write_all(path, &tokens)?;
        }
        Ok(())
    }

    pub fn set_session_token(broker_url: &str, token: &str) -> Result<(), String> {
        set_in(&store_path()?, broker_url, token)
    }

    pub fn get_session_token(broker_url: &str) -> Result<Option<String>, String> {
        get_in(&store_path()?, broker_url)
    }

    pub fn delete_session_token(broker_url: &str) -> Result<(), String> {
        delete_in(&store_path()?, broker_url)
    }
}

pub use imp::*;

#[cfg(all(test, not(target_os = "macos")))]
mod file_store_tests {
    use super::imp::{delete_in, get_in, set_in};

    const URL: &str = "https://broker.example.com";

    #[test]
    fn round_trip_overwrite_and_idempotent_delete() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("broker-sessions.json");

        assert_eq!(get_in(&path, URL).unwrap(), None);
        set_in(&path, URL, "secret-1").unwrap();
        set_in(&path, "https://other.example.com", "secret-x").unwrap();
        assert_eq!(get_in(&path, URL).unwrap().as_deref(), Some("secret-1"));

        set_in(&path, URL, "secret-2").unwrap();
        assert_eq!(get_in(&path, URL).unwrap().as_deref(), Some("secret-2"));

        delete_in(&path, URL).unwrap();
        delete_in(&path, URL).unwrap();
        assert_eq!(get_in(&path, URL).unwrap(), None);
        assert_eq!(
            get_in(&path, "https://other.example.com")
                .unwrap()
                .as_deref(),
            Some("secret-x")
        );
    }

    #[cfg(unix)]
    #[test]
    fn store_is_owner_only() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets").join("broker-sessions.json");
        set_in(&path, URL, "secret").unwrap();
        let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(path.parent().unwrap()), 0o700);
    }

    #[test]
    fn corrupt_store_is_an_error_not_a_silent_reset() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("broker-sessions.json");
        std::fs::write(&path, b"not json").unwrap();
        assert!(get_in(&path, URL).is_err());
        assert!(set_in(&path, URL, "secret").is_err());
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    const TEST_URL: &str = "https://test-broker.example.com/keychain-test";

    #[test]
    fn round_trip() {
        // Clean up from any prior failed run.
        let _ = delete_session_token(TEST_URL);

        set_session_token(TEST_URL, "test-secret-123").unwrap();
        let got = get_session_token(TEST_URL).unwrap();
        assert_eq!(got.as_deref(), Some("test-secret-123"));

        // Overwrite.
        set_session_token(TEST_URL, "test-secret-456").unwrap();
        let got = get_session_token(TEST_URL).unwrap();
        assert_eq!(got.as_deref(), Some("test-secret-456"));

        // Delete.
        delete_session_token(TEST_URL).unwrap();
        let got = get_session_token(TEST_URL).unwrap();
        assert!(got.is_none());

        // Delete again (idempotent).
        delete_session_token(TEST_URL).unwrap();
    }

    #[test]
    fn not_found_returns_none() {
        let got =
            get_session_token("https://nonexistent.example.com/no-such-keychain-entry").unwrap();
        assert!(got.is_none());
    }
}
