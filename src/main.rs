mod audio;
mod audit;
mod beatport;
mod changes;
mod cli;
mod color;
mod corpus;
mod db;
mod discogs;
mod eval_routing;
mod eval_tasks;
mod genre;
mod normalize;
mod store;
mod tags;
mod tools;
mod types;
mod xml;

use rmcp::ServiceExt;
use rmcp::transport::stdio;

fn should_run_cli<I, S>(mut args: I) -> bool
where
    I: Iterator<Item = S>,
    S: AsRef<str>,
{
    args.nth(1).is_some_and(|arg| {
        let a = arg.as_ref();
        matches!(
            a,
            "analyze" | "read-tags" | "write-tags" | "extract-art" | "embed-art"
        )
    })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if should_run_cli(std::env::args()) {
        tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .init();
        cli::main().await
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .init();
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
    fn runs_server_for_unrecognized_args() {
        assert!(!should_run_cli(
            vec!["reklawdbox", "--transport", "stdio"].into_iter()
        ));
    }
}
