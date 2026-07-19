use std::time::Instant;

use crate::adapters::audio;
use crate::application::cache_writer::serialize_cache_payload;

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

struct AnalysisSource {
    file_path: String,
    file_size: i64,
    file_mtime: i64,
}

trait AnalysisExecutors {
    async fn run_stratum(&self, file_path: &str) -> Result<StratumAnalysis, String>;

    async fn run_essentia(&self, file_path: &str) -> Option<Result<audio::EssentiaOutput, String>>;
}

struct RuntimeExecutors<'a> {
    essentia_python: Option<&'a str>,
}

impl AnalysisExecutors for RuntimeExecutors<'_> {
    async fn run_stratum(&self, file_path: &str) -> Result<StratumAnalysis, String> {
        let path_clone = file_path.to_string();
        let (samples, sample_rate) =
            tokio::task::spawn_blocking(move || audio::decode_to_samples(&path_clone))
                .await
                .map_err(|e| format!("Decode task failed: {e}"))?
                .map_err(|e| format!("Decode error: {e}"))?;
        let path_for_grid = file_path.to_string();
        tokio::task::spawn_blocking(move || {
            let input = load_rekordbox_grid_input_for_path(&path_for_grid);
            analyze_with_stratum_input(&samples, sample_rate, input)
        })
        .await
        .map_err(|e| format!("Analysis task failed: {e}"))?
        .map_err(|e| format!("Analysis error: {e}"))
    }

    async fn run_essentia(&self, file_path: &str) -> Option<Result<audio::EssentiaOutput, String>> {
        let python = self.essentia_python?;
        Some(
            audio::run_essentia(python, file_path)
                .await
                .map_err(|error| format!("Essentia error for {file_path}: {error}")),
        )
    }
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
    let source = AnalysisSource {
        file_path,
        file_size: metadata.len() as i64,
        file_mtime: file_mtime_unix(&metadata),
    };
    let executors = RuntimeExecutors { essentia_python };
    Ok(run_with_executors(
        source,
        needs_stratum,
        needs_essentia,
        continue_after_stratum_failure,
        &executors,
    )
    .await)
}

async fn run_with_executors(
    source: AnalysisSource,
    needs_stratum: bool,
    needs_essentia: bool,
    continue_after_stratum_failure: bool,
    executors: &impl AnalysisExecutors,
) -> AnalysisJobReport {
    let started = Instant::now();
    let mut cache_messages = Vec::with_capacity(2);
    let mut stratum = None;

    if needs_stratum {
        let result = match executors.run_stratum(&source.file_path).await {
            Ok(analysis) => {
                let StratumAnalysis {
                    result,
                    input_fingerprint,
                } = analysis;
                match serialize_cache_payload(&result, "stratum-dsp analysis") {
                    Ok(features_json) => {
                        cache_messages.push(AnalysisCacheWrite {
                            file_path: source.file_path.clone(),
                            analyzer: audio::ANALYZER_STRATUM.to_string(),
                            file_size: source.file_size,
                            file_mtime: source.file_mtime,
                            analyzer_version: audio::STRATUM_SCHEMA_VERSION.to_string(),
                            input_fingerprint,
                            features_json,
                        });
                        Ok(result)
                    }
                    Err(error) => Err(error),
                }
            }
            Err(error) => Err(error),
        };
        let failed = result.is_err();
        stratum = Some(result);
        if failed && !continue_after_stratum_failure {
            return AnalysisJobReport {
                stratum,
                essentia: None,
                cache_messages,
                elapsed_seconds: started.elapsed().as_secs_f64(),
            };
        }
    }

    let essentia = if needs_essentia {
        match executors.run_essentia(&source.file_path).await {
            Some(result) => match result {
                Ok(features) => match serialize_cache_payload(&features, "essentia analysis") {
                    Ok(features_json) => {
                        cache_messages.push(AnalysisCacheWrite {
                            file_path: source.file_path.clone(),
                            analyzer: audio::ANALYZER_ESSENTIA.to_string(),
                            file_size: source.file_size,
                            file_mtime: source.file_mtime,
                            analyzer_version: audio::ESSENTIA_SCHEMA_VERSION.to_string(),
                            input_fingerprint: String::new(),
                            features_json,
                        });
                        Some(Ok(features))
                    }
                    Err(error) => Some(Err(error)),
                },
                Err(error) => Some(Err(error)),
            },
            None => None,
        }
    } else {
        None
    };

    AnalysisJobReport {
        stratum,
        essentia,
        cache_messages,
        elapsed_seconds: started.elapsed().as_secs_f64(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct InjectedExecutors {
        stratum_calls: AtomicUsize,
        essentia_calls: AtomicUsize,
    }

    impl AnalysisExecutors for InjectedExecutors {
        async fn run_stratum(&self, _file_path: &str) -> Result<StratumAnalysis, String> {
            self.stratum_calls.fetch_add(1, Ordering::Relaxed);
            Err("injected Stratum failure".to_string())
        }

        async fn run_essentia(
            &self,
            _file_path: &str,
        ) -> Option<Result<audio::EssentiaOutput, String>> {
            self.essentia_calls.fetch_add(1, Ordering::Relaxed);
            Some(Ok(audio::EssentiaOutput::default()))
        }
    }

    fn synthetic_source() -> AnalysisSource {
        AnalysisSource {
            file_path: "/synthetic/track.flac".to_string(),
            file_size: 42,
            file_mtime: 84,
        }
    }

    #[tokio::test]
    async fn shared_analysis_job_preserves_partial_stratum_failure() {
        let continued_executors = InjectedExecutors::default();
        let continued = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            run_with_executors(synthetic_source(), true, true, true, &continued_executors),
        )
        .await
        .expect("continued analysis policy timed out");
        assert_eq!(
            continued.stratum.expect("Stratum ran").unwrap_err(),
            "injected Stratum failure"
        );
        assert!(continued.essentia.expect("Essentia ran").is_ok());
        assert_eq!(continued_executors.stratum_calls.load(Ordering::Relaxed), 1);
        assert_eq!(
            continued_executors.essentia_calls.load(Ordering::Relaxed),
            1
        );
        assert_eq!(continued.cache_messages.len(), 1);
        assert_eq!(
            continued.cache_messages[0].analyzer,
            audio::ANALYZER_ESSENTIA
        );
        assert_eq!(
            continued.cache_messages[0].file_path,
            "/synthetic/track.flac"
        );

        let stopped_executors = InjectedExecutors::default();
        let stopped = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            run_with_executors(synthetic_source(), true, true, false, &stopped_executors),
        )
        .await
        .expect("stop-on-Stratum-failure policy timed out");
        assert_eq!(
            stopped.stratum.expect("Stratum ran").unwrap_err(),
            "injected Stratum failure"
        );
        assert!(stopped.essentia.is_none());
        assert!(stopped.cache_messages.is_empty());
        assert_eq!(stopped_executors.stratum_calls.load(Ordering::Relaxed), 1);
        assert_eq!(stopped_executors.essentia_calls.load(Ordering::Relaxed), 0);
    }
}
