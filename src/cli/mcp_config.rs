use std::path::PathBuf;
use std::process::{Command, Stdio};

enum HostResult {
    Configured,
    AlreadyConfigured,
    NotDetected,
    Failed(String),
}

/// Resolve the binary path to use in MCP config. Normalizes Homebrew Cellar
/// paths to the stable `/opt/homebrew/bin/reklawdbox` symlink.
fn resolve_binary_path() -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| format!("Cannot determine binary path: {e}"))?;
    let canonical = std::fs::canonicalize(&exe).unwrap_or_else(|_| exe.clone());
    let path_str = canonical.to_string_lossy();

    if path_str.contains("/Cellar/reklawdbox/") {
        let stable = "/opt/homebrew/bin/reklawdbox";
        if std::fs::metadata(stable).is_ok() {
            return Ok(stable.to_string());
        }
    }
    Ok(exe.to_string_lossy().to_string())
}

/// Configure Claude Code by shelling out to `claude mcp add`.
fn configure_claude_code(binary_path: &str) -> HostResult {
    let version = match Command::new("claude")
        .arg("--version")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
    {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        Ok(_) | Err(_) => return HostResult::NotDetected,
    };

    let output = match Command::new("claude")
        .args(["mcp", "add", "-s", "user", "reklawdbox", "--", binary_path])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
    {
        Ok(o) => o,
        Err(e) => return HostResult::Failed(format!("failed to run claude: {e}")),
    };

    if output.status.success() {
        return HostResult::Configured;
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let msg = if !stderr.trim().is_empty() {
        stderr.trim()
    } else {
        stdout.trim()
    };

    if msg.contains("already exists") {
        return HostResult::AlreadyConfigured;
    }

    HostResult::Failed(format!("{msg} (claude {version})"))
}

fn claude_desktop_config_path() -> Option<PathBuf> {
    dirs::home_dir()
        .map(|h| h.join("Library/Application Support/Claude/claude_desktop_config.json"))
}

/// Configure Claude Desktop by writing to its config JSON.
fn configure_claude_desktop(binary_path: &str) -> HostResult {
    let config_path = match claude_desktop_config_path() {
        Some(p) => p,
        None => return HostResult::NotDetected,
    };

    if !config_path.parent().is_some_and(|p| p.exists()) {
        return HostResult::NotDetected;
    }

    let mut root: serde_json::Value = if config_path.exists() {
        let contents = match std::fs::read_to_string(&config_path) {
            Ok(c) => c,
            Err(e) => {
                return HostResult::Failed(format!("cannot read {}: {e}", config_path.display()));
            }
        };
        match serde_json::from_str(&contents) {
            Ok(v) => v,
            Err(e) => {
                return HostResult::Failed(format!("cannot parse {}: {e}", config_path.display()));
            }
        }
    } else {
        serde_json::json!({})
    };

    // If reklawdbox is already configured, don't touch it — user may have custom
    // args or env vars we shouldn't destroy.
    if root
        .get("mcpServers")
        .and_then(|s| s.get("reklawdbox"))
        .is_some()
    {
        return HostResult::AlreadyConfigured;
    }

    // Merge our entry
    let root_obj = match root.as_object_mut() {
        Some(o) => o,
        None => {
            return HostResult::Failed(format!(
                "{} has non-object root JSON",
                config_path.display()
            ))
        }
    };
    let servers = root_obj
        .entry("mcpServers")
        .or_insert_with(|| serde_json::json!({}));
    let servers_obj = match servers.as_object_mut() {
        Some(o) => o,
        None => {
            return HostResult::Failed(format!(
                "{} has non-object mcpServers field",
                config_path.display()
            ))
        }
    };
    servers_obj.insert(
        "reklawdbox".to_string(),
        serde_json::json!({ "command": binary_path }),
    );

    // Atomic write: write to temp file then rename
    let tmp_path = config_path.with_extension("setup-tmp");
    let json_str = match serde_json::to_string_pretty(&root) {
        Ok(s) => s,
        Err(e) => return HostResult::Failed(format!("JSON serialization failed: {e}")),
    };

    if let Err(e) = std::fs::write(&tmp_path, &json_str) {
        return HostResult::Failed(format!("cannot write {}: {e}", tmp_path.display()));
    }

    if let Err(e) = std::fs::rename(&tmp_path, &config_path) {
        // Clean up temp file on rename failure
        let _ = std::fs::remove_file(&tmp_path);
        return HostResult::Failed(format!("cannot rename to {}: {e}", config_path.display()));
    }

    HostResult::Configured
}

/// Detect and configure all supported MCP hosts. Non-fatal — prints warnings
/// on failure and always returns. Returns `true` if all attempted hosts
/// succeeded (or were already configured / not detected).
pub(crate) fn configure_mcp_hosts() -> bool {
    let binary_path = match resolve_binary_path() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("\nMCP configuration skipped: {e}");
            return false;
        }
    };

    println!();
    println!("Configuring MCP hosts...");

    let mut any_configured = false;
    let mut any_failed = false;

    // Claude Code
    match configure_claude_code(&binary_path) {
        HostResult::Configured => {
            println!("  Claude Code: configured ({binary_path})");
            any_configured = true;
        }
        HostResult::AlreadyConfigured => {
            println!("  Claude Code: already configured.");
        }
        HostResult::NotDetected => {
            println!("  Claude Code: not detected, skipping.");
            println!("    Run manually: claude mcp add -s user reklawdbox -- {binary_path}");
        }
        HostResult::Failed(msg) => {
            eprintln!("  Claude Code: failed — {msg}");
            eprintln!("    Run manually: claude mcp add -s user reklawdbox -- {binary_path}");
            any_failed = true;
        }
    }

    // Claude Desktop
    match configure_claude_desktop(&binary_path) {
        HostResult::Configured => {
            println!("  Claude Desktop: configured ({binary_path})");
            any_configured = true;
        }
        HostResult::AlreadyConfigured => {
            println!("  Claude Desktop: already configured.");
        }
        HostResult::NotDetected => {
            println!("  Claude Desktop: not detected, skipping.");
        }
        HostResult::Failed(msg) => {
            eprintln!("  Claude Desktop: failed — {msg}");
            any_failed = true;
        }
    }

    if any_configured {
        println!();
        println!(
            "Restart your MCP host to activate \
             (Claude Code: /mcp or new conversation, \
             Claude Desktop: quit and relaunch)."
        );
    }

    !any_failed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_binary_path_returns_a_path() {
        let path = resolve_binary_path().expect("should resolve");
        assert!(!path.is_empty());
    }

    #[test]
    fn claude_desktop_config_path_is_under_application_support() {
        if let Some(path) = claude_desktop_config_path() {
            let path_str = path.to_string_lossy();
            assert!(path_str.contains("Application Support/Claude"));
            assert!(path_str.ends_with("claude_desktop_config.json"));
        }
    }

    #[test]
    fn configure_claude_desktop_does_not_panic_with_nonexistent_binary() {
        let _ = configure_claude_desktop("/nonexistent/binary");
    }
}
