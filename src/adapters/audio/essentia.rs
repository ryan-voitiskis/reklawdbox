use std::io::Read;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration as StdDuration, Instant};

use serde::{Deserialize, Serialize};
use tokio::process::Command;
use tokio::time::{Duration as TokioDuration, timeout};

use super::AudioError;

const ESSENTIA_TIMEOUT_SECS: u64 = 300;

const ESSENTIA_SCRIPT: &str = include_str!("essentia_analysis.py");

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct EssentiaOutput {
    pub analyzer_version: String,
    pub danceability: Option<f64>,
    pub loudness_integrated: Option<f64>,
    pub loudness_range: Option<f64>,
    pub dynamic_complexity: Option<f64>,
    pub average_loudness: Option<f64>,
    pub bpm_essentia: Option<f64>,
    pub onset_rate: Option<f64>,
    pub rhythm_regularity: Option<f64>,
    pub spectral_centroid_mean: Option<f64>,
    pub dissonance_mean: Option<f64>,
    pub intensity_mean: Option<f64>,
    pub intensity_var: Option<f64>,
    pub mfcc_mean: Option<Vec<f64>>,
    pub mfcc_std: Option<Vec<f64>>,
    pub spectral_centroid_cv: Option<f64>,
    pub spectral_flux_mean: Option<f64>,
    pub spectral_flux_iqr: Option<f64>,
    pub spectral_contrast_mean: Option<Vec<f64>>,
}

pub(crate) fn parse_essentia_stdout(stdout: &[u8]) -> Result<EssentiaOutput, AudioError> {
    let text = std::str::from_utf8(stdout)
        .map_err(|e| AudioError::Parse(format!("Essentia stdout was not valid UTF-8: {e}")))?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(AudioError::Parse("Essentia stdout was empty".to_string()));
    }
    serde_json::from_str(trimmed)
        .map_err(|e| AudioError::Parse(format!("Failed to parse Essentia JSON output: {e}")))
}

pub async fn run_essentia(
    python_path: &str,
    audio_path: &str,
) -> Result<EssentiaOutput, AudioError> {
    let mut command = Command::new(python_path);
    command.args(["-c", ESSENTIA_SCRIPT, audio_path]);
    command.kill_on_drop(true);

    let output = timeout(
        TokioDuration::from_secs(ESSENTIA_TIMEOUT_SECS),
        command.output(),
    )
    .await
    .map_err(|_| {
        AudioError::Subprocess(format!(
            "Essentia analysis timed out after {ESSENTIA_TIMEOUT_SECS}s for '{audio_path}'"
        ))
    })?
    .map_err(|e| AudioError::Subprocess(format!("Failed to start Essentia subprocess: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stderr = if stderr.is_empty() {
            "(no stderr output)".to_string()
        } else {
            stderr
        };
        return Err(AudioError::Subprocess(format!(
            "Essentia subprocess failed for '{audio_path}': {stderr}"
        )));
    }

    parse_essentia_stdout(&output.stdout)
}

pub(crate) const ESSENTIA_PYTHON_ENV_VAR: &str = "CRATE_DIG_ESSENTIA_PYTHON";
pub(crate) const ESSENTIA_IMPORT_CHECK_SCRIPT: &str =
    "import essentia; print(essentia.__version__)";
pub(crate) const ESSENTIA_PROBE_TIMEOUT_SECS: u64 = 5;

pub(crate) fn validate_essentia_python(path: &str) -> bool {
    validate_essentia_python_with_timeout(path, StdDuration::from_secs(ESSENTIA_PROBE_TIMEOUT_SECS))
}

pub(crate) fn validate_essentia_python_with_timeout(path: &str, timeout: StdDuration) -> bool {
    let mut child = match std::process::Command::new(path)
        .args(["-c", ESSENTIA_IMPORT_CHECK_SCRIPT])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return false,
    };

    let Some(mut stdout_pipe) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return false;
    };
    let Some(mut stderr_pipe) = child.stderr.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return false;
    };

    let stdout_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout_pipe.read_to_end(&mut buf);
        buf
    });
    let stderr_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr_pipe.read_to_end(&mut buf);
        buf
    });

    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                std::thread::sleep(StdDuration::from_millis(25));
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };

    let stdout = stdout_handle.join().unwrap_or_default();
    let _stderr = stderr_handle.join().unwrap_or_default();

    let Some(status) = status else {
        return false;
    };
    if !status.success() {
        return false;
    }

    let stdout = String::from_utf8_lossy(&stdout);
    let version_line = stdout.lines().map(str::trim).find(|line| !line.is_empty());
    matches!(
        version_line,
        Some(line) if line.chars().any(|ch| ch.is_ascii_digit())
    )
}

pub(crate) fn probe_essentia_python_from_sources(
    env_override: Option<&str>,
    default_candidate: Option<PathBuf>,
) -> Option<String> {
    let mut candidates: Vec<String> = Vec::new();
    if let Some(path) = env_override.map(str::trim).filter(|path| !path.is_empty()) {
        candidates.push(path.to_string());
    }
    if let Some(path) = default_candidate {
        let path = path.to_string_lossy().to_string();
        if !path.is_empty() && !candidates.iter().any(|candidate| candidate == &path) {
            candidates.push(path);
        }
    }

    candidates
        .into_iter()
        .find(|candidate| validate_essentia_python(candidate))
}

pub(crate) fn probe_essentia_python_path() -> Option<String> {
    let env_override = std::env::var(ESSENTIA_PYTHON_ENV_VAR).ok();
    let default_candidate =
        dirs::home_dir().map(|home| home.join(".local/share/reklawdbox/essentia-venv/bin/python"));
    probe_essentia_python_from_sources(env_override.as_deref(), default_candidate)
}

pub(crate) const ESSENTIA_VENV_RELPATH: &str = ".local/share/reklawdbox/essentia-venv";

pub(crate) fn essentia_venv_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(ESSENTIA_VENV_RELPATH))
}

pub(crate) fn essentia_setup_hint() -> String {
    let mut checked = Vec::new();

    match std::env::var(ESSENTIA_PYTHON_ENV_VAR) {
        Ok(val) if !val.trim().is_empty() => {
            checked.push(format!(
                "env {ESSENTIA_PYTHON_ENV_VAR}={val} (not a valid Essentia Python)"
            ));
        }
        _ => {
            checked.push(format!("env {ESSENTIA_PYTHON_ENV_VAR} (not set)"));
        }
    }

    if let Some(venv_dir) = essentia_venv_dir() {
        let python_path = venv_dir.join("bin/python");
        if python_path.exists() {
            checked.push(format!(
                "{} (exists but Essentia import failed)",
                python_path.display()
            ));
        } else {
            checked.push(format!("{} (not found)", python_path.display()));
        }
    }

    format!(
        "Essentia not found. Checked: {}. Call the setup_essentia tool to install automatically.",
        checked.join(", ")
    )
}
