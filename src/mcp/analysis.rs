mod batch;
mod core;
mod coverage;
pub(crate) mod essentia;
mod handlers;
mod scan;
mod transport;

pub(super) use batch::BatchProgress;
pub(super) use core::{
    AudioCacheIdentity, audio_cache_identities_with_current_stratum_input, check_analysis_cache,
    get_fresh_analysis_entry, resolved_audio_cache_key,
};
pub(super) use coverage::handle_cache_coverage;
pub(super) use essentia::{
    ESSENTIA_IMPORT_CHECK_SCRIPT, essentia_setup_hint, essentia_venv_dir,
    probe_essentia_python_path, validate_essentia_python,
};
pub(super) use handlers::{
    AnalyzeAudioBatchOutput, handle_analyze_audio_batch, handle_analyze_track_audio,
    handle_setup_essentia,
};
pub(super) use scan::{resolve_file_path, scan_audio_directory};
pub(super) use transport::{AnalyzeAudioBatchParams, AnalyzeTrackAudioParams, CacheCoverageParams};

#[cfg(test)]
pub(super) use core::{
    audio_cache_identities_with_fingerprint_loader, audio_cache_identity,
    audio_cache_identity_with_stratum_input_fingerprint, check_analysis_cache_for_identity,
};
#[cfg(test)]
pub(super) use essentia::{
    probe_essentia_python_from_sources, validate_essentia_python_with_timeout,
};
