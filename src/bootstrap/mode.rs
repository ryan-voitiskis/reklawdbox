//! Pure launch-mode selection for the CLI and stdio MCP surfaces.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LaunchMode {
    Cli,
    McpStdio,
}

pub(crate) fn detect<I, S>(mut args: I, stdin_is_terminal: bool) -> LaunchMode
where
    I: Iterator<Item = S>,
    S: AsRef<str>,
{
    let first_argument = args.nth(1);
    if stdin_is_terminal
        || first_argument
            .as_ref()
            .is_some_and(|argument| crate::cli::command::recognizes_cli_argument(argument.as_ref()))
    {
        LaunchMode::Cli
    } else {
        LaunchMode::McpStdio
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    fn routes_piped_to_cli(args: &[&str]) -> bool {
        detect(args.iter().copied(), false) == LaunchMode::Cli
    }

    #[test]
    fn launch_mode_matrix_preserves_cli_and_piped_mcp() {
        let mut command = crate::cli::command::Cli::command();
        command.build();

        for subcommand in command.get_subcommands() {
            assert_eq!(
                detect(["reklawdbox", subcommand.get_name()].into_iter(), false),
                LaunchMode::Cli,
                "subcommand {} must route piped stdin to CLI",
                subcommand.get_name()
            );
            for alias in subcommand.get_all_aliases() {
                assert_eq!(
                    detect(["reklawdbox", alias].into_iter(), false),
                    LaunchMode::Cli,
                    "alias {alias} must route piped stdin to CLI"
                );
            }
        }

        for flag in ["--help", "-h", "--version", "-V"] {
            assert_eq!(
                detect(["reklawdbox", flag].into_iter(), false),
                LaunchMode::Cli,
                "root flag {flag} must route piped stdin to CLI"
            );
        }

        let cases = [
            (vec!["reklawdbox"], true, LaunchMode::Cli),
            (vec!["reklawdbox"], false, LaunchMode::McpStdio),
            (vec!["reklawdbox", "unknown"], true, LaunchMode::Cli),
            (vec!["reklawdbox", "unknown"], false, LaunchMode::McpStdio),
            (
                vec!["reklawdbox", "--transport", "stdio"],
                false,
                LaunchMode::McpStdio,
            ),
        ];
        for (args, stdin_is_terminal, expected) in cases {
            assert_eq!(detect(args.into_iter(), stdin_is_terminal), expected);
        }
    }

    #[test]
    fn runs_server_when_no_subcommand_is_given() {
        assert!(!routes_piped_to_cli(&["reklawdbox"]));
    }

    #[test]
    fn runs_cli_for_analyze_subcommand() {
        assert!(routes_piped_to_cli(&["reklawdbox", "analyze"]));
    }

    #[test]
    fn runs_cli_for_hydrate_subcommand() {
        assert!(routes_piped_to_cli(&["reklawdbox", "hydrate"]));
    }

    #[test]
    fn runs_cli_for_read_tags_subcommand() {
        assert!(routes_piped_to_cli(&["reklawdbox", "read-tags"]));
    }

    #[test]
    fn runs_cli_for_write_tags_subcommand() {
        assert!(routes_piped_to_cli(&["reklawdbox", "write-tags"]));
    }

    #[test]
    fn runs_cli_for_extract_art_subcommand() {
        assert!(routes_piped_to_cli(&["reklawdbox", "extract-art"]));
    }

    #[test]
    fn runs_cli_for_embed_art_subcommand() {
        assert!(routes_piped_to_cli(&["reklawdbox", "embed-art"]));
    }

    #[test]
    fn runs_cli_for_backup_subcommand() {
        assert!(routes_piped_to_cli(&["reklawdbox", "backup"]));
    }

    #[test]
    fn runs_cli_for_setup_subcommand() {
        assert!(routes_piped_to_cli(&["reklawdbox", "setup"]));
    }

    #[test]
    fn runs_cli_for_help_and_version_flags() {
        for flag in ["--help", "-h", "help", "--version", "-V"] {
            assert!(
                routes_piped_to_cli(&["reklawdbox", flag]),
                "{flag} should route to CLI"
            );
        }
    }

    #[test]
    fn runs_server_for_unrecognized_args() {
        assert!(!routes_piped_to_cli(&[
            "reklawdbox",
            "--transport",
            "stdio"
        ]));
    }
}
