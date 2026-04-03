use std::path::PathBuf;

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

fn music_dir_mcp_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join("Music").join(".mcp.json"))
}

/// Configure Claude Code by writing `.mcp.json` to `~/Music/`.
///
/// This scopes reklawdbox tools to Claude Code sessions started from `~/Music`,
/// preventing tool pollution in unrelated projects. Previous versions registered
/// at user scope (`claude mcp add -s user`), which loaded tools everywhere.
fn configure_claude_code(binary_path: &str) -> HostResult {
    let mcp_path = match music_dir_mcp_path() {
        Some(p) => p,
        None => return HostResult::Failed("cannot determine home directory".to_string()),
    };

    if !mcp_path.parent().is_some_and(std::path::Path::exists) {
        return HostResult::Failed(format!(
            "{} does not exist — create it or add reklawdbox to a project .mcp.json manually.\n\
             See https://reklawdbox.com/getting-started/",
            mcp_path.parent().unwrap().display()
        ));
    }

    let mut root: serde_json::Value = if mcp_path.exists() {
        let contents = match std::fs::read_to_string(&mcp_path) {
            Ok(c) => c,
            Err(e) => {
                return HostResult::Failed(format!("cannot read {}: {e}", mcp_path.display()));
            }
        };
        match serde_json::from_str(&contents) {
            Ok(v) => v,
            Err(e) => {
                return HostResult::Failed(format!("cannot parse {}: {e}", mcp_path.display()));
            }
        }
    } else {
        serde_json::json!({})
    };

    // If reklawdbox is already configured, don't touch it — user may have custom
    // args or env vars we shouldn't destroy. Warn if the command path differs.
    if let Some(existing) = root.get("mcpServers").and_then(|s| s.get("reklawdbox")) {
        if let Some(existing_cmd) = existing.get("command").and_then(|c| c.as_str())
            && existing_cmd != binary_path {
                eprintln!(
                    "    Note: ~/Music/.mcp.json points to {existing_cmd},\n    \
                     but this binary is {binary_path}.\n    \
                     Edit ~/Music/.mcp.json to update it."
                );
            }
        return HostResult::AlreadyConfigured;
    }

    // Merge our entry
    let root_obj = match root.as_object_mut() {
        Some(o) => o,
        None => {
            return HostResult::Failed(format!(
                "{} has non-object root JSON",
                mcp_path.display()
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
                mcp_path.display()
            ))
        }
    };
    servers_obj.insert(
        "reklawdbox".to_string(),
        serde_json::json!({ "command": binary_path }),
    );

    // Atomic write: write to temp file then rename
    let tmp_path = mcp_path.with_extension("setup-tmp");
    let json_str = match serde_json::to_string_pretty(&root) {
        Ok(s) => s,
        Err(e) => return HostResult::Failed(format!("JSON serialization failed: {e}")),
    };

    if let Err(e) = std::fs::write(&tmp_path, &json_str) {
        return HostResult::Failed(format!("cannot write {}: {e}", tmp_path.display()));
    }

    if let Err(e) = std::fs::rename(&tmp_path, &mcp_path) {
        let _ = std::fs::remove_file(&tmp_path);
        return HostResult::Failed(format!("cannot rename to {}: {e}", mcp_path.display()));
    }

    HostResult::Configured
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

    if !config_path.parent().is_some_and(std::path::Path::exists) {
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

/// Check whether `~/.claude.json` has a global `mcpServers.reklawdbox` entry.
/// This indicates a stale registration from a previous setup (<= v0.19) that
/// causes reklawdbox tools to appear in every Claude Code project.
///
/// Returns `Ok(true)` if stale, `Ok(false)` if clean, `Err` if the check
/// itself failed (file unreadable, malformed JSON).
fn has_stale_global_registration() -> Result<bool, String> {
    let path = match dirs::home_dir() {
        Some(h) => h.join(".claude.json"),
        None => return Ok(false),
    };
    if !path.exists() {
        return Ok(false);
    }
    let contents = std::fs::read_to_string(&path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let root: serde_json::Value = serde_json::from_str(&contents)
        .map_err(|e| format!("cannot parse {}: {e}", path.display()))?;
    Ok(root
        .get("mcpServers")
        .and_then(|s| s.get("reklawdbox"))
        .is_some())
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
    let mut cc_ok = false;

    // Claude Code (~/Music/.mcp.json — scoped to music sessions only)
    match configure_claude_code(&binary_path) {
        HostResult::Configured => {
            println!("  Claude Code: configured (~/Music/.mcp.json)");
            any_configured = true;
            cc_ok = true;
        }
        HostResult::AlreadyConfigured => {
            println!("  Claude Code: already configured.");
            cc_ok = true;
        }
        HostResult::NotDetected => {}
        HostResult::Failed(msg) => {
            eprintln!("  Claude Code: failed — {msg}");
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

    // Warn about stale global registration from previous setup versions
    match has_stale_global_registration() {
        Ok(true) => {
            eprintln!();
            eprintln!("  Warning: found global reklawdbox in ~/.claude.json");
            eprintln!("  This causes tools to load in every Claude Code project.");
            eprintln!("  Remove it: claude mcp remove -s user reklawdbox");
        }
        Err(msg) => {
            eprintln!();
            eprintln!("  Warning: could not check ~/.claude.json for stale entries: {msg}");
        }
        Ok(false) => {}
    }

    if any_configured {
        println!();
        println!(
            "Restart your MCP host to activate \
             (Claude Code: /mcp or new conversation, \
             Claude Desktop: quit and relaunch)."
        );
    }

    if cc_ok && any_configured {
        println!();
        println!("Start Claude Code from ~/Music to access reklawdbox tools.");
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
    fn music_dir_mcp_path_is_under_music() {
        if let Some(path) = music_dir_mcp_path() {
            let path_str = path.to_string_lossy();
            assert!(path_str.contains("/Music/"));
            assert!(path_str.ends_with(".mcp.json"));
        }
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
