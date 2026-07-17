use std::io::{self, Write};

use crate::adapters::platform::config;
use crate::adapters::providers::discogs;
use crate::adapters::rekordbox as db;
use crate::application::analysis::setup::{EssentiaSetupStatus, setup_essentia};

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
    let setup = setup_essentia()?;
    match setup.status {
        EssentiaSetupStatus::AlreadyInstalled => println!(
            "Essentia is already installed at {}",
            setup.runtime.python_path
        ),
        EssentiaSetupStatus::Installed => {
            println!("Essentia installed at {}", setup.runtime.python_path);
            if let Some(ref python) = setup.python_bin_used {
                println!("Using Python: {python}");
            }
        }
    }
    println!(
        "Managed analyzer contract: {}",
        setup.runtime.analyzer_contract
    );
    println!(
        "Runtime manifest: CPython {}; Essentia distribution {} (module {}); NumPy {}; PyYAML {}; six {}",
        setup.runtime.python_version,
        setup.runtime.essentia_version,
        setup.runtime.essentia_module_version,
        setup.runtime.numpy_version,
        setup.runtime.pyyaml_version,
        setup.runtime.six_version,
    );
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
