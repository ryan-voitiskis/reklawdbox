//! Clap command declaration and command-surface metadata.

use std::path::Path;

use clap::{CommandFactory, Parser};

use crate::adapters::state as store;

#[derive(Parser)]
#[command(
    name = "reklawdbox",
    version,
    about = "Rekordbox library management — MCP server + CLI tools",
    after_help = "When invoked without arguments over a piped stdin, reklawdbox starts as an MCP server (stdio transport)."
)]
pub(crate) enum Cli {
    /// Batch audio analysis (stratum-dsp + Essentia)
    Analyze(super::analyze::AnalyzeArgs),
    /// Manage Rekordbox library backups
    Backup(super::backup::BackupArgs),
    /// Batch enrichment + analysis (Discogs and audio analysis)
    Hydrate(super::hydrate::HydrateArgs),
    /// Read metadata tags from audio files
    ReadTags(super::tags::ReadTagsArgs),
    /// Write metadata tags to audio files
    WriteTags(super::tags::WriteTagsArgs),
    /// Extract embedded cover art from an audio file
    ExtractArt(super::tags::ExtractArtArgs),
    /// Embed cover art into audio files
    EmbedArt(super::tags::EmbedArtArgs),
    /// Install Essentia and configure reklawdbox
    Setup(super::setup::SetupArgs),
    /// Clear the stored Discogs broker session (forces re-auth on next lookup)
    DisconnectBroker,
}

pub(crate) fn recognizes_cli_argument(argument: &str) -> bool {
    let mut command = Cli::command();
    command.build();

    command.get_subcommands().any(|subcommand| {
        subcommand.get_name() == argument
            || subcommand.get_all_aliases().any(|alias| alias == argument)
    }) || command.get_arguments().any(|option| {
        option
            .get_long()
            .is_some_and(|long| argument.strip_prefix("--") == Some(long))
            || option.get_short().is_some_and(|short| {
                let mut chars = argument.chars();
                chars.next() == Some('-') && chars.next() == Some(short) && chars.next().is_none()
            })
    })
}

pub(crate) async fn run() -> Result<(), Box<dyn std::error::Error>> {
    eprintln!(
        "{}",
        console::style(format!("reklawdbox v{}", env!("CARGO_PKG_VERSION"))).dim()
    );
    match Cli::parse() {
        Cli::Analyze(args) => super::analyze::run_analyze(args).await,
        Cli::Backup(args) => super::backup::run_backup(args).await,
        Cli::Hydrate(args) => super::hydrate::run_hydrate(args).await,
        Cli::ReadTags(args) => super::tags::run_read_tags(args),
        Cli::WriteTags(args) => super::tags::run_write_tags(args),
        Cli::ExtractArt(args) => super::tags::run_extract_art(args),
        Cli::EmbedArt(args) => super::tags::run_embed_art(args),
        Cli::Setup(args) => super::setup::run_setup(args),
        Cli::DisconnectBroker => disconnect_broker(),
    }
}

fn disconnect_broker() -> Result<(), Box<dyn std::error::Error>> {
    let cfg = match crate::adapters::providers::discogs::BrokerConfig::from_env() {
        crate::adapters::providers::discogs::BrokerConfigStatus::Ok(cfg) => cfg,
        crate::adapters::providers::discogs::BrokerConfigStatus::InvalidUrl(url) => {
            eprintln!("invalid broker URL: {url}");
            return Ok(());
        }
    };
    let store_path = store::resolve_path();
    let conn = store::open(store_path_as_utf8(&store_path)?)?;
    store::clear_broker_discogs_session(&conn, &cfg.base_url)?;
    eprintln!("broker session cleared for {}", cfg.base_url);
    Ok(())
}

fn store_path_as_utf8(path: &Path) -> Result<&str, std::io::Error> {
    path.to_str().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "internal state database path is not valid UTF-8",
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn non_utf8_store_path_is_rejected_without_fallback() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let path =
            std::path::PathBuf::from(OsString::from_vec(vec![b'/', b't', b'm', b'p', b'/', 0xff]));
        let err = store_path_as_utf8(&path).expect_err("non-UTF-8 path should be rejected");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }

    #[test]
    fn top_level_help_preserves_commands_and_after_help() {
        let mut command = Cli::command();
        command.build();
        assert_eq!(
            command
                .get_subcommands()
                .map(clap::Command::get_name)
                .collect::<Vec<_>>(),
            [
                "analyze",
                "backup",
                "hydrate",
                "read-tags",
                "write-tags",
                "extract-art",
                "embed-art",
                "setup",
                "disconnect-broker",
                "help",
            ]
        );
        assert_eq!(
            command.get_after_help().map(ToString::to_string).as_deref(),
            Some(
                "When invoked without arguments over a piped stdin, reklawdbox starts as an MCP server (stdio transport)."
            )
        );
    }
}
