use std::time::Instant;

use crate::adapters::audio;

use super::batch::serialize_cache_payload;
use super::identity::{
    analyze_with_stratum_input, file_mtime_unix, load_rekordbox_grid_input_for_path,
};
use super::model::{AnalysisCacheWrite, StratumAnalysis};

#[derive(Debug)]
pub(crate) struct AnalysisJobReport {
    pub(crate) stratum: Option<Result<audio::StratumResult, String>>,
    pub(crate) essentia: Option<Result<audio::EssentiaOutput, String>>,
    pub(crate) cache_messages: Vec<AnalysisCacheWrite>,
    pub(crate) elapsed_seconds: f64,
}

pub(crate) async fn run(
    raw_file_path: &str,
    needs_stratum: bool,
    needs_essentia: bool,
    essentia_python: Option<&str>,
    continue_after_stratum_failure: bool,
) -> Result<AnalysisJobReport, String> {
    if !needs_stratum && !needs_essentia {
        return Err("Nothing to analyze".to_string());
    }
    let file_path =
        audio::resolve_audio_path(raw_file_path).map_err(|_| "File not found".to_string())?;
    let metadata = std::fs::metadata(&file_path).map_err(|e| format!("Cannot stat file: {e}"))?;
    let file_size = metadata.len() as i64;
    let file_mtime = file_mtime_unix(&metadata);
    let started = Instant::now();
    let mut cache_messages = Vec::with_capacity(2);
    let mut stratum = None;

    if needs_stratum {
        let path_clone = file_path.clone();
        let result = async {
            let (samples, sample_rate) =
                tokio::task::spawn_blocking(move || audio::decode_to_samples(&path_clone))
                    .await
                    .map_err(|e| format!("Decode task failed: {e}"))?
                    .map_err(|e| format!("Decode error: {e}"))?;
            let path_for_grid = file_path.clone();
            let analysis = tokio::task::spawn_blocking(move || {
                let input = load_rekordbox_grid_input_for_path(&path_for_grid);
                analyze_with_stratum_input(&samples, sample_rate, input)
            })
            .await
            .map_err(|e| format!("Analysis task failed: {e}"))?
            .map_err(|e| format!("Analysis error: {e}"))?;
            let StratumAnalysis {
                result,
                input_fingerprint,
            } = analysis;
            let features_json = serialize_cache_payload(&result, "stratum-dsp analysis")?;
            cache_messages.push(AnalysisCacheWrite {
                file_path: file_path.clone(),
                analyzer: audio::ANALYZER_STRATUM.to_string(),
                file_size,
                file_mtime,
                analyzer_version: audio::STRATUM_SCHEMA_VERSION.to_string(),
                input_fingerprint,
                features_json,
            });
            Ok::<_, String>(result)
        }
        .await;
        let failed = result.is_err();
        stratum = Some(result);
        if failed && !continue_after_stratum_failure {
            return Ok(AnalysisJobReport {
                stratum,
                essentia: None,
                cache_messages,
                elapsed_seconds: started.elapsed().as_secs_f64(),
            });
        }
    }

    let essentia = if needs_essentia {
        match essentia_python {
            Some(python) => match audio::run_essentia(python, &file_path).await {
                Ok(features) => match serialize_cache_payload(&features, "essentia analysis") {
                    Ok(features_json) => {
                        cache_messages.push(AnalysisCacheWrite {
                            file_path: file_path.clone(),
                            analyzer: audio::ANALYZER_ESSENTIA.to_string(),
                            file_size,
                            file_mtime,
                            analyzer_version: audio::ESSENTIA_SCHEMA_VERSION.to_string(),
                            input_fingerprint: String::new(),
                            features_json,
                        });
                        Some(Ok(features))
                    }
                    Err(error) => Some(Err(error)),
                },
                Err(error) => Some(Err(format!("Essentia error for {file_path}: {error}"))),
            },
            None => None,
        }
    } else {
        None
    };

    Ok(AnalysisJobReport {
        stratum,
        essentia,
        cache_messages,
        elapsed_seconds: started.elapsed().as_secs_f64(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_analysis_job_preserves_partial_stratum_failure() {
        let report = AnalysisJobReport {
            stratum: Some(Err("stratum failed".to_string())),
            essentia: Some(Ok(audio::EssentiaOutput::default())),
            cache_messages: Vec::new(),
            elapsed_seconds: 0.0,
        };
        assert!(report.stratum.is_some_and(|result| result.is_err()));
        assert!(report.essentia.is_some_and(|result| result.is_ok()));
    }
}
