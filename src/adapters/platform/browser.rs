//! Best-effort "open this URL in the user's browser".
//!
//! Callers always print the URL as well, so failing to launch a browser (a
//! headless server, a container) is not an error.

/// Candidate launchers, in order. Under WSL `wslview` (wslu) or
/// `explorer.exe` hand the URL to the Windows browser; `xdg-open` covers
/// desktop Linux.
fn launchers() -> &'static [&'static str] {
    if cfg!(target_os = "macos") {
        &["open"]
    } else if cfg!(windows) {
        &["explorer.exe"]
    } else if is_wsl() {
        &["wslview", "explorer.exe", "xdg-open"]
    } else {
        &["xdg-open"]
    }
}

pub(crate) fn is_wsl() -> bool {
    std::env::var_os("WSL_DISTRO_NAME").is_some()
        || std::fs::read_to_string("/proc/sys/kernel/osrelease")
            .is_ok_and(|release| release.to_ascii_lowercase().contains("microsoft"))
}

/// Returns true when some launcher process started.
pub(crate) fn open_url(url: &str) -> bool {
    launchers().iter().any(|program| {
        std::process::Command::new(program)
            .arg(url)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .is_ok()
    })
}
