mod batch;
mod coverage;
mod handlers;
mod scan;
mod transport;

pub(super) use batch::BatchProgress;
pub(super) use coverage::handle_cache_coverage;
pub(super) use handlers::{
    AnalyzeAudioBatchOutput, handle_analyze_audio_batch, handle_analyze_track_audio,
    handle_setup_essentia,
};
#[cfg(test)]
pub(in crate::mcp) use handlers::{handle_setup_essentia_with, setup_essentia_payload};
pub(super) use scan::{resolve_file_path, scan_audio_directory};
pub(super) use transport::{AnalyzeAudioBatchParams, AnalyzeTrackAudioParams, CacheCoverageParams};
