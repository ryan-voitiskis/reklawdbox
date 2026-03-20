mod audio;
mod audit;
mod bandcamp;
mod beatport;
mod changes;
mod classify;
mod cli;
mod color;
mod config;
mod corpus;
mod db;
mod discogs;
mod eval_routing;
mod eval_tasks;
mod genre;
mod keychain;
mod musicbrainz;
mod normalize;
mod store;
mod tags;
mod tools;
mod types;
mod xml;

use std::io::IsTerminal;
use std::path::PathBuf;
use std::sync::OnceLock;

use rmcp::ServiceExt;
use rmcp::transport::stdio;

/// Resolve the project root directory.
///
/// Checks, in order:
/// 1. `REKLAWDBOX_PROJECT_ROOT` env var
/// 2. Ancestor of the running binary (works when binary is at `target/{profile}/reklawdbox`)
/// 3. Current working directory
pub(crate) fn project_root() -> &'static PathBuf {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        if let Ok(root) = std::env::var("REKLAWDBOX_PROJECT_ROOT") {
            let p = PathBuf::from(&root);
            if p.join("Cargo.toml").exists() {
                return p;
            }
        }
        if let Ok(exe) = std::env::current_exe() {
            // Binary at {root}/target/{profile}/reklawdbox → root is 3 parents up
            if let Some(root) = exe
                .parent()
                .and_then(|p| p.parent())
                .and_then(|p| p.parent())
            {
                if root.join("Cargo.toml").exists() {
                    return root.to_path_buf();
                }
            }
        }
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    })
}

/// Load env vars from `.mcp.json` so CLI commands get the same config
/// that Claude Code injects for the MCP server. Shell env takes precedence.
fn load_env_from_mcp_json() {
    let Ok(bytes) = std::fs::read(project_root().join(".mcp.json")) else {
        return;
    };
    let Ok(root) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return;
    };
    let Some(env) = root
        .pointer("/mcpServers/reklawdbox/env")
        .and_then(|v| v.as_object())
    else {
        return;
    };
    for (key, val) in env {
        if let Some(val) = val.as_str() {
            if std::env::var_os(key).is_none() {
                // SAFETY: called at the top of main() before any threads are spawned.
                unsafe { std::env::set_var(key, val) };
            }
        }
    }
}

fn should_run_cli<I, S>(mut args: I) -> bool
where
    I: Iterator<Item = S>,
    S: AsRef<str>,
{
    args.nth(1).is_some_and(|arg| {
        let a = arg.as_ref();
        matches!(
            a,
            "analyze"
                | "hydrate"
                | "read-tags"
                | "write-tags"
                | "extract-art"
                | "embed-art"
                | "setup"
                | "--help"
                | "-h"
                | "help"
                | "--version"
                | "-V"
        )
    })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Both functions only set vars not already present, so call order
    // determines priority: shell env > .mcp.json > config.toml.
    load_env_from_mcp_json();
    // SAFETY: same context as load_env_from_mcp_json above.
    unsafe { config::load_into_env() };
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    if should_run_cli(std::env::args()) || std::io::stdin().is_terminal() {
        cli::main().await
    } else {
        let server = tools::ReklawdboxServer::new(db::resolve_db_path());
        let service = server.serve(stdio()).await?;
        service.waiting().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::should_run_cli;

    #[test]
    fn runs_server_when_no_subcommand_is_given() {
        assert!(!should_run_cli(vec!["reklawdbox"].into_iter()));
    }

    #[test]
    fn runs_cli_for_analyze_subcommand() {
        assert!(should_run_cli(vec!["reklawdbox", "analyze"].into_iter()));
    }

    #[test]
    fn runs_cli_for_hydrate_subcommand() {
        assert!(should_run_cli(vec!["reklawdbox", "hydrate"].into_iter()));
    }

    #[test]
    fn runs_cli_for_read_tags_subcommand() {
        assert!(should_run_cli(vec!["reklawdbox", "read-tags"].into_iter()));
    }

    #[test]
    fn runs_cli_for_write_tags_subcommand() {
        assert!(should_run_cli(vec!["reklawdbox", "write-tags"].into_iter()));
    }

    #[test]
    fn runs_cli_for_extract_art_subcommand() {
        assert!(should_run_cli(
            vec!["reklawdbox", "extract-art"].into_iter()
        ));
    }

    #[test]
    fn runs_cli_for_embed_art_subcommand() {
        assert!(should_run_cli(vec!["reklawdbox", "embed-art"].into_iter()));
    }

    #[test]
    fn runs_cli_for_setup_subcommand() {
        assert!(should_run_cli(vec!["reklawdbox", "setup"].into_iter()));
    }

    #[test]
    fn runs_cli_for_help_and_version_flags() {
        for flag in ["--help", "-h", "help", "--version", "-V"] {
            assert!(
                should_run_cli(vec!["reklawdbox", flag].into_iter()),
                "{flag} should route to CLI"
            );
        }
    }

    #[test]
    fn runs_server_for_unrecognized_args() {
        assert!(!should_run_cli(
            vec!["reklawdbox", "--transport", "stdio"].into_iter()
        ));
    }
}
