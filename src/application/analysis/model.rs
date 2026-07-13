/// The exact Rekordbox beat-grid snapshot selected for one Stratum analysis,
/// paired with the cache identity derived from that same snapshot.
#[derive(Debug, Clone)]
pub struct RekordboxGridInput {
    pub grid: Option<stratum_dsp::BeatGrid>,
    pub fingerprint: String,
}

#[derive(Debug)]
pub struct StratumAnalysis {
    pub result: crate::adapters::audio::StratumResult,
    pub input_fingerprint: String,
}

/// One analyzer cache write produced by the shared analysis job.
#[derive(Debug, Clone)]
pub struct AnalysisCacheWrite {
    pub file_path: String,
    pub analyzer: String,
    pub file_size: i64,
    pub file_mtime: i64,
    pub analyzer_version: String,
    pub input_fingerprint: String,
    pub features_json: String,
}
