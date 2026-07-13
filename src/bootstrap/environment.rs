//! Early environment discovery and configuration loading.

use std::path::PathBuf;
use std::sync::OnceLock;

/// Checks, in order:
/// 1. `REKLAWDBOX_PROJECT_ROOT` env var
/// 2. Ancestor of the running binary (works when binary is at `target/{profile}/reklawdbox`)
/// 3. Current working directory
pub(crate) fn project_root() -> &'static PathBuf {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        if let Ok(root) = std::env::var("REKLAWDBOX_PROJECT_ROOT") {
            let path = PathBuf::from(&root);
            if path.join("Cargo.toml").exists() {
                return path;
            }
        }
        if let Ok(executable) = std::env::current_exe() {
            // Binary at {root}/target/{profile}/reklawdbox → root is 3 parents up.
            if let Some(root) = executable
                .parent()
                .and_then(|path| path.parent())
                .and_then(|path| path.parent())
                && root.join("Cargo.toml").exists()
            {
                return root.to_path_buf();
            }
        }
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    })
}

/// Load env vars from `.mcp.json` so CLI commands get the same config
/// that Claude Code injects for the MCP server. Shell env takes precedence.
pub(crate) fn load_env_from_mcp_json() {
    let Ok(bytes) = std::fs::read(project_root().join(".mcp.json")) else {
        return;
    };
    let Ok(root) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return;
    };
    let Some(env) = root
        .pointer("/mcpServers/reklawdbox/env")
        .and_then(|value| value.as_object())
    else {
        return;
    };
    for (key, value) in env {
        if let Some(value) = value.as_str()
            && std::env::var_os(key).is_none()
        {
            // SAFETY: only sets vars that are unset (is_none guard), and runs
            // before any application code reads these vars. Tokio worker threads
            // exist but do not access env during this early init phase.
            unsafe { std::env::set_var(key, value) };
        }
    }
}
