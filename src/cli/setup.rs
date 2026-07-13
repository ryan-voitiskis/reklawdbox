use std::io::{self, Write};
use std::process::{Command, Stdio};

use crate::adapters::audio::{essentia_venv_dir, validate_essentia_python};
use crate::config;
use crate::db;
use crate::discogs;

const PYTHON_CANDIDATES: &[&str] = &[
    "python3.13",
    "python3.12",
    "python3.11",
    "python3.10",
    "python3.9",
    "python3",
];

#[derive(clap::Args)]
pub(crate) struct SetupArgs {
    /// Configure a custom Discogs broker URL and token (self-hosted brokers only).
    #[arg(long)]
    broker: bool,

    /// Accept defaults without prompting (only applies to --broker).
    #[arg(long, short = 'y')]
    yes: bool,
}

pub(crate) fn run_setup(args: SetupArgs) -> Result<(), Box<dyn std::error::Error>> {
    install_essentia()?;

    let mcp_ok = super::mcp_config::configure_mcp_hosts();

    if args.broker {
        configure_broker(args.yes)?;
    }

    verify_rekordbox_db();

    println!();
    if mcp_ok {
        println!("Setup complete.");
    } else {
        println!("Setup complete (MCP configuration had errors — see above).");
    }
    Ok(())
}

fn install_essentia() -> Result<(), Box<dyn std::error::Error>> {
    let venv_dir = essentia_venv_dir()
        .ok_or("Cannot determine home directory for venv location — is $HOME set?")?;
    let venv_python = venv_dir.join("bin/python");
    let venv_python_str = venv_python.to_string_lossy().to_string();

    if venv_python.exists() && validate_essentia_python(&venv_python_str) {
        println!("Essentia is already installed at {venv_python_str}");
        return Ok(());
    }

    let python_bin = find_python()?;
    println!("Using Python: {python_bin}");

    if let Some(parent) = venv_dir.parent() {
        let display = parent.display();
        std::fs::create_dir_all(parent).map_err(|e| format!("Failed to create {display}: {e}"))?;
    }

    // --clear ensures a fresh start if a broken venv exists
    println!("Creating virtualenv: {}", venv_dir.display());
    run_cmd(
        &python_bin,
        &["-m", "venv", "--clear", &venv_dir.to_string_lossy()],
        "venv creation",
    )?;

    // Verify pip exists (missing when ensurepip is unavailable, e.g. Debian without python3-venv)
    let venv_pip = venv_dir.join("bin/pip");
    if !venv_pip.exists() {
        return Err(format!(
            "Venv was created but pip is missing at {}. \
             On Debian/Ubuntu, install the python3-venv package.",
            venv_pip.display()
        )
        .into());
    }

    println!("Installing Essentia (this may take a minute)...");
    run_cmd(
        venv_pip.to_string_lossy().as_ref(),
        &["install", "--pre", "essentia"],
        "pip install essentia",
    )?;

    validate_essentia_import(&venv_python_str)?;

    println!("Essentia installed at {venv_python_str}");
    Ok(())
}

fn validate_essentia_import(python_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let output = Command::new(python_path)
        .args(["-c", "import essentia; print(essentia.__version__)"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("Failed to run Essentia import check: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "Essentia installed but import validation failed (exit {}):\n{}",
            output.status,
            stderr.trim()
        )
        .into());
    }
    Ok(())
}

fn find_python() -> Result<String, Box<dyn std::error::Error>> {
    let mut diagnostics: Vec<String> = Vec::new();
    for &candidate in PYTHON_CANDIDATES {
        match Command::new(candidate)
            .args([
                "-c",
                "import sys; raise SystemExit(0 if sys.version_info >= (3, 9) else 1)",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
        {
            Ok(output) if output.status.success() => return Ok(candidate.to_string()),
            Ok(output) => {
                diagnostics.push(format!("  {candidate}: version too old or check failed"));
                let stderr = String::from_utf8_lossy(&output.stderr);
                if !stderr.trim().is_empty() {
                    diagnostics.push(format!("    {}", stderr.trim()));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                diagnostics.push(format!("  {candidate}: not found"));
            }
            Err(e) => {
                diagnostics.push(format!("  {candidate}: {e}"));
            }
        }
    }
    Err(format!(
        "No supported Python found (3.9+ required). Tried:\n{}",
        diagnostics.join("\n")
    )
    .into())
}

fn run_cmd(program: &str, args: &[&str], context: &str) -> Result<(), Box<dyn std::error::Error>> {
    let output = Command::new(program)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("Failed to run {context}: {e}"))?;

    let stderr = String::from_utf8_lossy(&output.stderr);

    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut msg = format!("{context} failed (exit {}):", output.status);
        if !stderr.trim().is_empty() {
            msg.push_str(&format!("\n{}", stderr.trim()));
        }
        if !stdout.trim().is_empty() {
            msg.push_str(&format!("\n{}", stdout.trim()));
        }
        return Err(msg.into());
    }

    // Surface warnings from successful runs (pip deprecation notices, etc.)
    if !stderr.trim().is_empty() {
        eprintln!("{}", stderr.trim());
    }
    Ok(())
}

fn verify_rekordbox_db() {
    println!();
    let path = match db::resolve_db_path() {
        Some(p) => p,
        None => {
            eprintln!(
                "Warning: Rekordbox database not found. \
                 Set REKORDBOX_DB_PATH if your database is in a non-standard location."
            );
            return;
        }
    };
    match db::open(&path) {
        Ok(conn) => {
            let count = db::active_track_count(&conn).unwrap_or(0);
            println!("Rekordbox database: {count} tracks ({path})");
        }
        Err(e) => {
            eprintln!("Warning: Found {path} but could not open it: {e}");
        }
    }
}

fn configure_broker(accept_defaults: bool) -> Result<(), Box<dyn std::error::Error>> {
    let config_path =
        config::config_path().ok_or("Could not determine config directory — is $HOME set?")?;

    let mut cfg = config::load();

    let current_url = cfg
        .discogs
        .broker
        .url
        .as_deref()
        .unwrap_or(discogs::DEFAULT_BROKER_URL);
    if accept_defaults {
        if cfg.discogs.broker.url.is_none() {
            cfg.discogs.broker.url = Some(discogs::DEFAULT_BROKER_URL.to_string());
        }
    } else {
        println!();
        println!("Discogs broker URL [{current_url}]:");
        let input = read_line_trimmed()?;
        cfg.discogs.broker.url = Some(if input.is_empty() {
            current_url.to_string()
        } else {
            input
        });
    }

    if let Some(ref url) = cfg.discogs.broker.url
        && discogs::normalize_base_url(url).is_none()
    {
        return Err(format!("Invalid URL: {url}").into());
    }

    // Broker token — only configured interactively for custom broker URLs.
    let is_default_url = cfg
        .discogs
        .broker
        .url
        .as_deref()
        .and_then(discogs::normalize_base_url)
        .as_deref()
        == Some(discogs::DEFAULT_BROKER_URL);

    if !is_default_url && !accept_defaults {
        println!();
        println!(
            "Custom broker URL detected. Set the token required by that broker if it enforces client-token auth."
        );
        let show_token = if cfg.discogs.broker.token.is_some() {
            "(configured)"
        } else {
            "(none)"
        };
        println!("Discogs broker token [{show_token}]:");
        let input = read_line_trimmed()?;
        if !input.is_empty() {
            let cleaned = input.trim_matches('"').trim_matches('\'').to_string();
            if cleaned.is_empty() {
                return Err("Token cannot be empty.".into());
            }
            cfg.discogs.broker.token = Some(cleaned);
        }
    }

    if !is_default_url && cfg.discogs.broker.token.is_none() {
        eprintln!(
            "Warning: Custom broker URL is configured but no broker token is set. \
             Set {} or re-run setup without --yes to configure it.",
            discogs::BROKER_TOKEN_ENV
        );
    }

    config::save(&cfg)?;
    println!("Broker config saved to {}", config_path.display());
    Ok(())
}

fn read_line_trimmed() -> io::Result<String> {
    print!("> ");
    io::stdout().flush()?;
    let mut buf = String::new();
    io::stdin().read_line(&mut buf)?;
    Ok(buf.trim().to_string())
}
