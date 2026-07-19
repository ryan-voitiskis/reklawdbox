use super::script::PreparedScript;

/// Execute the embedded script interactively with inherited standard streams.
pub(crate) async fn execute_embedded_interactive(args: &[&str]) -> Result<(), String> {
    let script = PreparedScript::embedded().map_err(|error| error.to_string())?;
    let path = script.path().to_string_lossy().to_string();
    let args: Vec<String> = args.iter().map(|argument| argument.to_string()).collect();

    let status = tokio::task::spawn_blocking(move || {
        std::process::Command::new("bash")
            .arg(&path)
            .args(&args)
            .status()
    })
    .await
    .map_err(|error| format!("backup task failed: {error}"))?
    .map_err(|error| format!("backup launch failed: {error}"))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "backup failed with exit status {}",
            status
                .code()
                .map_or_else(|| "signal".to_string(), |code| code.to_string()),
        ))
    }
}
