#[derive(clap::Args)]
pub(crate) struct BackupArgs {
    /// Backup database files only (~50MB, fast)
    #[arg(long)]
    db_only: bool,

    /// List existing backups
    #[arg(long)]
    list: bool,

    /// Restore from a backup archive
    #[arg(long, value_name = "PATH")]
    restore: Option<String>,
}

pub(crate) async fn run_backup(args: BackupArgs) -> Result<(), Box<dyn std::error::Error>> {
    let result = if args.list {
        crate::backup::execute_embedded_interactive(&["--list"]).await
    } else if let Some(ref path) = args.restore {
        crate::backup::execute_embedded_interactive(&["--restore", path]).await
    } else if args.db_only {
        crate::backup::execute_embedded_interactive(&["--db-only"]).await
    } else {
        crate::backup::execute_embedded_interactive(&[]).await
    };
    result.map_err(|e| e.into())
}
