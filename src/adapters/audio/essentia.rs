use serde::{Deserialize, Serialize};
use tokio::process::Command;
use tokio::time::{Duration as TokioDuration, timeout};

use super::{
    AudioError,
    essentia_environment::{
        ESSENTIA_CONTRACT_ID, ESSENTIA_PYTHON_ENV_VAR, SUPPORTED_ESSENTIA_VERSION,
        SUPPORTED_NUMPY_VERSION, SUPPORTED_PYTHON_PREFIX, SUPPORTED_PYYAML_VERSION,
        SUPPORTED_SIX_VERSION, essentia_venv_dir,
    },
};

const ESSENTIA_TIMEOUT_SECS: u64 = 300;

const ESSENTIA_SCRIPT: &str = include_str!("essentia_analysis.py");

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct EssentiaOutput {
    pub analyzer_version: String,
    /// Complete managed runtime identity; persisted output from schema 3 on.
    pub runtime_manifest: Option<EssentiaRuntimeManifest>,
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct EssentiaRuntimeManifest {
    pub python_version: String,
    pub python_implementation: String,
    pub essentia_version: String,
    pub numpy_version: String,
    pub pyyaml_version: String,
    pub six_version: String,
    pub analyzer_contract: String,
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

pub(crate) fn validate_runtime_manifest(output: &EssentiaOutput) -> Result<(), AudioError> {
    let manifest = output.runtime_manifest.as_ref().ok_or_else(|| {
        AudioError::Analysis(
            "Essentia output omitted the required schema-v3 runtime manifest".into(),
        )
    })?;
    let supported = output.analyzer_version == SUPPORTED_ESSENTIA_VERSION
        && manifest.python_implementation == "cpython"
        && manifest.python_version.starts_with(SUPPORTED_PYTHON_PREFIX)
        && manifest.essentia_version == SUPPORTED_ESSENTIA_VERSION
        && manifest.numpy_version == SUPPORTED_NUMPY_VERSION
        && manifest.pyyaml_version == SUPPORTED_PYYAML_VERSION
        && manifest.six_version == SUPPORTED_SIX_VERSION
        && manifest.analyzer_contract == ESSENTIA_CONTRACT_ID;
    if supported {
        return Ok(());
    }
    Err(AudioError::Analysis(format!(
        "Essentia runtime changed after setup; refusing schema-v3 cache write (analyzer={}, implementation={}, python={}, essentia={}, numpy={}, pyyaml={}, six={}, contract={})",
        output.analyzer_version,
        manifest.python_implementation,
        manifest.python_version,
        manifest.essentia_version,
        manifest.numpy_version,
        manifest.pyyaml_version,
        manifest.six_version,
        manifest.analyzer_contract,
    )))
}

pub async fn run_essentia(
    python_path: &str,
    audio_path: &str,
) -> Result<EssentiaOutput, AudioError> {
    let mut command = Command::new(python_path);
    command.args(["-c", ESSENTIA_SCRIPT, audio_path]);
    command.env("REKLAWDBOX_ESSENTIA_CONTRACT", ESSENTIA_CONTRACT_ID);
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

    let parsed = parse_essentia_stdout(&output.stdout)?;
    validate_runtime_manifest(&parsed)?;
    Ok(parsed)
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
