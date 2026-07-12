mod analysis_job;
mod analyze;
mod backup;
mod hydrate;
mod mcp_config;
mod setup;
mod tags;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use clap::Parser;
use indicatif::MultiProgress;
use tokio_util::sync::CancellationToken;

use crate::{audio, store};

/// Spawn signal handlers for graceful shutdown (Ctrl+C) and terminal resize
/// (SIGWINCH). The SIGWINCH handler forces indicatif to clear-and-redraw so its
/// internal line tracking stays in sync with the actual terminal state.
pub(crate) fn spawn_signal_handlers(
    mp: &MultiProgress,
    cancel: &CancellationToken,
    cancellation_state: &CliCancellationState,
) {
    let cancel_clone = cancel.clone();
    let state_clone = cancellation_state.clone();
    let mp_clone = mp.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            state_clone.mark_user_requested();
            mp_clone
                .println("Shutting down gracefully... (waiting for in-flight tasks)")
                .ok();
            cancel_clone.cancel();
        }
    });

    // Register before spawning so failures surface here rather than panicking
    // inside a detached task where tokio silently swallows the panic.
    let sig = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::window_change()) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("Could not register SIGWINCH handler (resize redraw disabled): {e}");
            return;
        }
    };
    let winch_cancel = cancel.clone();
    let winch_mp = mp.clone();
    tokio::spawn(async move {
        let mut sig = sig;
        loop {
            tokio::select! {
                _ = sig.recv() => { winch_mp.println("").ok(); }
                _ = winch_cancel.cancelled() => break,
            }
        }
    });
}

/// Tracks cancellation that came from the operator rather than internal or
/// normal end-of-batch shutdown.
#[derive(Clone, Debug, Default)]
pub(crate) struct CliCancellationState {
    user_requested: Arc<AtomicBool>,
}

impl CliCancellationState {
    pub(crate) fn mark_user_requested(&self) {
        self.user_requested.store(true, Ordering::Release);
    }

    pub(crate) fn user_requested(&self) -> bool {
        self.user_requested.load(Ordering::Acquire)
    }
}

#[derive(Clone, Copy, Debug, Default, clap::ValueEnum)]
pub(crate) enum CpuPreset {
    /// ~50% of cores, niced — use while working on the machine
    #[default]
    Background,
    /// Max cores (cores - 2), no nicing — for overnight/unattended runs
    Overnight,
}

impl std::fmt::Display for CpuPreset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CpuPreset::Background => write!(f, "background"),
            CpuPreset::Overnight => write!(f, "overnight"),
        }
    }
}

pub(crate) fn analysis_concurrency_for_preset(preset: CpuPreset) -> usize {
    let cpus = std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(4);
    match preset {
        CpuPreset::Background => (cpus / 2).clamp(2, 16) as usize,
        CpuPreset::Overnight => cpus.saturating_sub(2).clamp(2, 16) as usize,
    }
}

pub(crate) fn apply_cpu_niceness(preset: CpuPreset) {
    if matches!(preset, CpuPreset::Background) {
        // SAFETY: setpriority with PRIO_PROCESS/0 targets the calling process.
        // Raising niceness (lowering priority) always succeeds for unprivileged users.
        unsafe {
            libc::setpriority(libc::PRIO_PROCESS, 0, 10);
        }
    }
}

pub(crate) fn cpu_preset_summary(preset: CpuPreset, concurrency: usize) -> String {
    match preset {
        CpuPreset::Background => format!("CPU: background ({concurrency} cores, niced)"),
        CpuPreset::Overnight => format!("CPU: overnight ({concurrency} cores)"),
    }
}

/// Empirically measured stratum-dsp memory usage per minute of audio, plus 20% margin.
const MEMORY_MB_PER_MINUTE: u32 = 600;

/// Fixed overhead per analysis task (buffers, symphonia decoder, etc.).
const MEMORY_FIXED_OVERHEAD_MB: u32 = 200;

/// Minimum cost charged per track (short tracks still need decode buffers).
const MEMORY_MIN_COST_MB: u32 = 500;

/// Fraction of system RAM available for overnight analysis.
const OVERNIGHT_MEMORY_FRACTION: f64 = 0.75;

/// Fraction of system RAM available for background analysis.
const BACKGROUND_MEMORY_FRACTION: f64 = 0.30;

/// Abort the pipeline after this many consecutive cache write failures.
pub(crate) const MAX_CONSECUTIVE_CACHE_WRITE_FAILURES: u32 = 3;

/// Falls back to 16 GB if sysctl fails.
fn system_total_memory_mb() -> u32 {
    #[cfg(target_os = "macos")]
    {
        use std::mem::MaybeUninit;
        let mut size: usize = std::mem::size_of::<u64>();
        let mut value = MaybeUninit::<u64>::uninit();
        let name = c"hw.memsize";
        // SAFETY: sysctlbyname with valid name, correctly-sized output buffer.
        let ret = unsafe {
            libc::sysctlbyname(
                name.as_ptr(),
                value.as_mut_ptr().cast(),
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        if ret == 0 {
            // SAFETY: sysctlbyname succeeded, value is initialised.
            let bytes = unsafe { value.assume_init() };
            return (bytes / (1024 * 1024)) as u32;
        }
        tracing::warn!("sysctlbyname(hw.memsize) failed — falling back to 16 GB memory assumption");
    }
    #[cfg(not(target_os = "macos"))]
    tracing::warn!("No sysctl on this platform — falling back to 16 GB memory assumption");
    16_384
}

pub(crate) fn memory_budget_mb(preset: CpuPreset) -> u32 {
    let total = system_total_memory_mb();
    let fraction = match preset {
        CpuPreset::Background => BACKGROUND_MEMORY_FRACTION,
        CpuPreset::Overnight => OVERNIGHT_MEMORY_FRACTION,
    };
    ((total as f64 * fraction) as u32).max(MEMORY_MIN_COST_MB)
}

pub(crate) fn track_memory_cost_mb(duration_secs: i32) -> u32 {
    let minutes = (duration_secs.max(0) as f64) / 60.0;
    let cost = (minutes * MEMORY_MB_PER_MINUTE as f64) as u32 + MEMORY_FIXED_OVERHEAD_MB;
    cost.max(MEMORY_MIN_COST_MB)
}

pub(crate) fn memory_preset_summary(budget_mb: u32) -> String {
    let total_mb = system_total_memory_mb();
    format!(
        "Memory: {:.1} GB budget ({:.0}% of {:.1} GB)",
        budget_mb as f64 / 1024.0,
        (budget_mb as f64 / total_mb as f64) * 100.0,
        total_mb as f64 / 1024.0,
    )
}

#[derive(Parser)]
#[command(
    name = "reklawdbox",
    version,
    about = "Rekordbox library management — MCP server + CLI tools",
    after_help = "When invoked without arguments over a piped stdin, reklawdbox starts as an MCP server (stdio transport)."
)]
enum Cli {
    /// Batch audio analysis (stratum-dsp + Essentia)
    Analyze(analyze::AnalyzeArgs),
    /// Manage Rekordbox library backups
    Backup(backup::BackupArgs),
    /// Batch enrichment + analysis (Discogs, Beatport, audio analysis)
    Hydrate(hydrate::HydrateArgs),
    /// Read metadata tags from audio files
    ReadTags(tags::ReadTagsArgs),
    /// Write metadata tags to audio files
    WriteTags(tags::WriteTagsArgs),
    /// Extract embedded cover art from an audio file
    ExtractArt(tags::ExtractArtArgs),
    /// Embed cover art into audio files
    EmbedArt(tags::EmbedArtArgs),
    /// Install Essentia and configure reklawdbox
    Setup(setup::SetupArgs),
    /// Clear the stored Discogs broker session (forces re-auth on next lookup)
    DisconnectBroker,
}

pub async fn main() -> Result<(), Box<dyn std::error::Error>> {
    eprintln!(
        "{}",
        console::style(format!("reklawdbox v{}", env!("CARGO_PKG_VERSION"))).dim()
    );
    let cli = Cli::parse();
    match cli {
        Cli::Analyze(args) => analyze::run_analyze(args).await,
        Cli::Backup(args) => backup::run_backup(args).await,
        Cli::Hydrate(args) => hydrate::run_hydrate(args).await,
        Cli::ReadTags(args) => tags::run_read_tags(args),
        Cli::WriteTags(args) => tags::run_write_tags(args),
        Cli::ExtractArt(args) => tags::run_extract_art(args),
        Cli::EmbedArt(args) => tags::run_embed_art(args),
        Cli::Setup(args) => setup::run_setup(args),
        Cli::DisconnectBroker => {
            let cfg = match crate::discogs::BrokerConfig::from_env() {
                crate::discogs::BrokerConfigStatus::Ok(cfg) => cfg,
                crate::discogs::BrokerConfigStatus::InvalidUrl(url) => {
                    eprintln!("invalid broker URL: {url}");
                    return Ok(());
                }
            };
            let store_path = store::resolve_path();
            let conn = store::open(store_path_as_utf8(&store_path)?)?;
            store::clear_broker_discogs_session(&conn, &cfg.base_url)?;
            eprintln!("broker session cleared for {}", cfg.base_url);
            Ok(())
        }
    }
}

fn store_path_as_utf8(path: &Path) -> Result<&str, std::io::Error> {
    path.to_str().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "internal state database path is not valid UTF-8",
        )
    })
}

fn file_mtime_unix(metadata: &std::fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs() as i64)
}

fn is_cache_fresh(
    cached: Option<&store::CachedAudioAnalysis>,
    schema_version: &str,
    file_size: i64,
    file_mtime: i64,
    input_fingerprint: &str,
) -> bool {
    store::is_audio_analysis_fresh(
        cached,
        schema_version,
        file_size,
        file_mtime,
        input_fingerprint,
    )
}

struct CacheProbe {
    cache_key: String,
    file_size: i64,
    file_mtime: i64,
    stratum_input_fingerprint: String,
}

impl CacheProbe {
    fn input_fingerprint_for_analyzer(&self, analyzer: &str) -> &str {
        if analyzer == audio::ANALYZER_STRATUM {
            &self.stratum_input_fingerprint
        } else {
            ""
        }
    }
}

fn cache_probe_for_path(file_path: &str, skip_cached: bool) -> Option<CacheProbe> {
    if !skip_cached {
        return None;
    }
    let cache_key = audio::resolve_audio_path(file_path).ok()?;
    let metadata = std::fs::metadata(&cache_key).ok()?;
    let stratum_input_fingerprint =
        audio::load_rekordbox_grid_input_for_path(&cache_key).fingerprint;
    Some(CacheProbe {
        cache_key,
        file_size: metadata.len() as i64,
        file_mtime: file_mtime_unix(&metadata),
        stratum_input_fingerprint,
    })
}

fn has_fresh_cache_entry(
    store_conn: &rusqlite::Connection,
    cache_probe: Option<&CacheProbe>,
    analyzer: &str,
    schema_version: &str,
) -> Result<bool, rusqlite::Error> {
    if let Some(cache_probe) = cache_probe {
        let cached = store::get_audio_analysis(store_conn, &cache_probe.cache_key, analyzer)?;
        Ok(is_cache_fresh(
            cached.as_ref(),
            schema_version,
            cache_probe.file_size,
            cache_probe.file_mtime,
            cache_probe.input_fingerprint_for_analyzer(analyzer),
        ))
    } else {
        Ok(false)
    }
}

fn cache_status_for_track(
    store_conn: &rusqlite::Connection,
    cache_probe: Option<&CacheProbe>,
    skip_cached: bool,
    essentia_available: bool,
) -> Result<(bool, bool), rusqlite::Error> {
    let has_stratum = if skip_cached {
        has_fresh_cache_entry(
            store_conn,
            cache_probe,
            audio::ANALYZER_STRATUM,
            audio::STRATUM_SCHEMA_VERSION,
        )?
    } else {
        false
    };

    let has_essentia = if !essentia_available {
        true
    } else if skip_cached {
        has_fresh_cache_entry(
            store_conn,
            cache_probe,
            audio::ANALYZER_ESSENTIA,
            audio::ESSENTIA_SCHEMA_VERSION,
        )?
    } else {
        false
    };

    Ok((has_stratum, has_essentia))
}

#[derive(Debug)]
pub(crate) struct CliCacheWriteMsg {
    pub file_path: String,
    pub analyzer: String,
    pub file_size: i64,
    pub file_mtime: i64,
    pub analyzer_version: String,
    pub input_fingerprint: String,
    pub features_json: String,
}

pub(crate) fn persist_cli_cache_message(
    conn: &rusqlite::Connection,
    message: &CliCacheWriteMsg,
) -> Result<(), rusqlite::Error> {
    store::set_audio_analysis_with_fingerprint(
        conn,
        &message.file_path,
        &message.analyzer,
        message.file_size,
        message.file_mtime,
        &message.analyzer_version,
        &message.input_fingerprint,
        &message.features_json,
    )
}

pub(crate) struct CacheWriteRequest<T> {
    pub payload: T,
    pub acknowledgement: tokio::sync::oneshot::Sender<Result<(), String>>,
}

#[derive(Debug, Default)]
pub(crate) struct CacheWriterReport {
    pub attempted: u32,
    pub succeeded: u32,
    pub failed: u32,
    pub threshold_cancelled: bool,
    pub error_summaries: Vec<String>,
}

impl CacheWriterReport {
    pub(crate) fn record_success(&mut self) {
        self.attempted += 1;
        self.succeeded += 1;
    }

    pub(crate) fn record_failure(&mut self, summary: String) {
        self.attempted += 1;
        self.failed += 1;
        if self.error_summaries.len() < 10 && !self.error_summaries.contains(&summary) {
            self.error_summaries.push(summary);
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CliBatchFailure {
    pub command: &'static str,
    pub track_or_provider_failures: u32,
    pub worker_join_failures: u32,
    pub writer_failures: u32,
    pub incomplete: usize,
    pub user_cancelled: bool,
    pub error_summaries: Vec<String>,
}

impl std::fmt::Display for CliBatchFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} batch failed: {} track/provider failures, {} task join failures, {} cache write failures, {} incomplete",
            self.command,
            self.track_or_provider_failures,
            self.worker_join_failures,
            self.writer_failures,
            self.incomplete,
        )?;
        if self.user_cancelled {
            write!(f, ", cancelled by user")?;
        }
        if !self.error_summaries.is_empty() {
            write!(f, ": {}", self.error_summaries.join("; "))?;
        }
        Ok(())
    }
}

impl std::error::Error for CliBatchFailure {}

pub(crate) fn cli_batch_outcome(
    command: &'static str,
    track_or_provider_failures: u32,
    worker_join_failures: u32,
    writer_failures: u32,
    incomplete: usize,
    user_cancelled: bool,
    error_summaries: Vec<String>,
) -> Result<(), CliBatchFailure> {
    if track_or_provider_failures == 0
        && worker_join_failures == 0
        && writer_failures == 0
        && incomplete == 0
        && !user_cancelled
    {
        Ok(())
    } else {
        Err(CliBatchFailure {
            command,
            track_or_provider_failures,
            worker_join_failures,
            writer_failures,
            incomplete,
            user_cancelled,
            error_summaries,
        })
    }
}

pub(crate) fn task_join_error_summary(task: &str, error: &tokio::task::JoinError) -> String {
    if error.is_cancelled() {
        format!("{task} was cancelled")
    } else if error.is_panic() {
        format!("{task} panicked")
    } else {
        format!("{task} failed")
    }
}

pub(crate) fn serialize_cache_payload<T: serde::Serialize>(
    value: &T,
    context: &str,
) -> Result<String, String> {
    serde_json::to_string(value).map_err(|e| format!("{context} cache serialization failed: {e}"))
}

pub(crate) async fn send_cache_message<T>(
    tx: &tokio::sync::mpsc::Sender<CacheWriteRequest<T>>,
    message: T,
    context: &str,
) -> Result<(), String> {
    let (acknowledgement, result) = tokio::sync::oneshot::channel();
    tx.send(CacheWriteRequest {
        payload: message,
        acknowledgement,
    })
    .await
    .map_err(|e| format!("{context} cache queue send failed: {e}"))?;
    result
        .await
        .map_err(|_| format!("{context} cache acknowledgement canceled"))?
}

fn is_audio_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| {
            crate::audio::AUDIO_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str())
        })
}

fn expand_paths(paths: &[String], recursive: bool) -> Vec<PathBuf> {
    let mut result = Vec::new();
    for p in paths {
        let path = PathBuf::from(p);
        if path.is_dir() {
            collect_audio_files(&path, recursive, &mut result);
        } else {
            result.push(path);
        }
    }
    result
}

fn collect_audio_files(dir: &Path, recursive: bool, result: &mut Vec<PathBuf>) {
    match std::fs::read_dir(dir) {
        Ok(entries) => {
            let mut files = Vec::new();
            let mut subdirs = Vec::new();
            for entry in entries.filter_map(std::result::Result::ok) {
                let is_symlink = entry.file_type().is_ok_and(|ft| ft.is_symlink());
                let path = entry.path();
                if path.is_file() && is_audio_file(&path) {
                    files.push(path);
                } else if recursive && path.is_dir() && !is_symlink {
                    subdirs.push(path);
                }
            }
            files.sort();
            result.extend(files);
            subdirs.sort();
            for subdir in subdirs {
                collect_audio_files(&subdir, true, result);
            }
        }
        Err(e) => {
            eprintln!(
                "Warning: skipping unreadable directory {}: {e}",
                dir.display()
            );
        }
    }
}

/// "album_artist" -> "Album Artist", "bpm" -> "BPM", etc.
fn display_field_name(field: &str) -> String {
    field
        .split('_')
        .map(|word| match word {
            "bpm" => "BPM".to_string(),
            _ => {
                let mut chars = word.chars();
                match chars.next() {
                    Some(c) => {
                        let upper: String = c.to_uppercase().collect();
                        format!("{upper}{}", chars.as_str())
                    }
                    None => String::new(),
                }
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
pub(crate) mod async_test_support {
    use std::future::Future;
    use std::time::Duration;

    pub(crate) const STEP_TIMEOUT: Duration = Duration::from_secs(1);
    pub(crate) const TEST_WATCHDOG: Duration = Duration::from_secs(5);

    pub(crate) struct TaskGuard<T> {
        handle: Option<tokio::task::JoinHandle<T>>,
    }

    impl<T> TaskGuard<T> {
        pub(crate) fn new(handle: tokio::task::JoinHandle<T>) -> Self {
            Self {
                handle: Some(handle),
            }
        }

        pub(crate) async fn join_raw(
            mut self,
            context: &str,
        ) -> Result<Result<T, tokio::task::JoinError>, String> {
            let mut handle = self.handle.take().expect("task guard handle");
            match tokio::time::timeout(STEP_TIMEOUT, &mut handle).await {
                Ok(result) => Ok(result),
                Err(_) => {
                    handle.abort();
                    tokio::time::timeout(STEP_TIMEOUT, &mut handle)
                        .await
                        .map_err(|_| format!("{context} cleanup timed out"))
                        .map(|_| ())?;
                    Err(format!("{context} timed out"))
                }
            }
        }

        pub(crate) async fn join(self, context: &str) -> Result<T, String> {
            self.join_raw(context)
                .await?
                .map_err(|error| format!("{context} failed: {error}"))
        }

        pub(crate) fn abort(&self) {
            if let Some(handle) = &self.handle {
                handle.abort();
            }
        }
    }

    impl<T> Drop for TaskGuard<T> {
        fn drop(&mut self) {
            if let Some(handle) = &self.handle {
                handle.abort();
            }
        }
    }

    pub(crate) async fn bounded<F: Future>(future: F, context: &str) -> Result<F::Output, String> {
        tokio::time::timeout(STEP_TIMEOUT, future)
            .await
            .map_err(|_| format!("{context} timed out"))
    }
}

#[cfg(test)]
mod tests {
    use super::analyze::{handle_analysis_result, handle_decode_result, mark_track_outcome};
    use super::async_test_support::{TEST_WATCHDOG, TaskGuard, bounded};
    use super::{
        CacheProbe, CacheWriteRequest, CliCacheWriteMsg, CliCancellationState,
        cache_status_for_track, cli_batch_outcome, file_mtime_unix, is_cache_fresh,
        persist_cli_cache_message, send_cache_message, serialize_cache_payload, store_path_as_utf8,
    };
    use crate::{
        audio, audio::AudioError, audio::StratumResult, store, store::CachedAudioAnalysis,
    };
    use serde::ser::Error as _;
    use std::time::Duration;

    fn cached(file_size: i64, file_mtime: i64) -> CachedAudioAnalysis {
        CachedAudioAnalysis {
            file_path: "/tmp/a.flac".to_string(),
            analyzer: "stratum-dsp".to_string(),
            file_size,
            file_mtime,
            analysis_version: audio::STRATUM_SCHEMA_VERSION.to_string(),
            input_fingerprint: audio::STRATUM_HMM_INPUT_FINGERPRINT.to_string(),
            features_json: "{}".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_store_path_is_rejected_without_fallback() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let path =
            std::path::PathBuf::from(OsString::from_vec(vec![b'/', b't', b'm', b'p', b'/', 0xff]));
        let err = store_path_as_utf8(&path).expect_err("non-UTF-8 path should be rejected");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }

    fn sample_stratum_result() -> StratumResult {
        StratumResult {
            bpm: 120.0,
            bpm_confidence: 0.9,
            key: "Am".to_string(),
            key_camelot: "8A".to_string(),
            key_confidence: 0.8,
            key_clarity: 0.7,
            grid_stability: 0.95,
            grid_source: "hmm".to_string(),
            duration_seconds: 180.0,
            processing_time_ms: 42.0,
            analyzer_version: "1.0.0".to_string(),
            mod_centroid: Some(12.5),
            harmonic_proportion: Some(0.65),
            decay_mid_tau: Some(180.0),
            decay_mid_r2: Some(0.92),
            decay_high_tau: Some(95.0),
            decay_high_r2: Some(0.88),
            dub_stab_onset_count: None,
            dub_stab_onset_rate: None,
            dub_stab_rate_basis: None,
            dub_stab_histogram: None,
            dub_stab_template: None,
            dub_stab_template_score: None,
            kick_pattern: None,
            kick_pattern_confidence: None,
            kick_kicks_per_bar: None,
            kick_onset_count: None,
            kick_rate_basis: None,
            kick_histogram: None,
            sections: None,
            flags: vec![],
            warnings: vec![],
        }
    }

    fn cache_message() -> CliCacheWriteMsg {
        CliCacheWriteMsg {
            file_path: "/tmp/test.flac".to_string(),
            analyzer: "stratum-dsp".to_string(),
            file_size: 1,
            file_mtime: 2,
            analyzer_version: "v1".to_string(),
            input_fingerprint: audio::STRATUM_HMM_INPUT_FINGERPRINT.to_string(),
            features_json: "{}".to_string(),
        }
    }

    struct FailingSerialize;

    impl serde::Serialize for FailingSerialize {
        fn serialize<S>(&self, _serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            Err(S::Error::custom("boom"))
        }
    }

    fn open_temp_store_with_probe() -> (tempfile::TempDir, rusqlite::Connection, CacheProbe) {
        let dir = tempfile::tempdir().expect("temp dir");

        let audio_path = dir.path().join("track.wav");
        std::fs::write(&audio_path, b"not-a-real-audio-file").expect("write audio fixture");

        let metadata = std::fs::metadata(&audio_path).expect("metadata");
        let cache_key = audio_path.to_string_lossy().to_string();
        let file_size = metadata.len() as i64;
        let file_mtime = file_mtime_unix(&metadata);
        let probe = CacheProbe {
            cache_key,
            file_size,
            file_mtime,
            stratum_input_fingerprint: audio::STRATUM_HMM_INPUT_FINGERPRINT.to_string(),
        };

        let store_path = dir.path().join("cache.sqlite3");
        let conn = store::open(store_path.to_str().expect("utf-8 path")).expect("open store");
        (dir, conn, probe)
    }

    #[test]
    fn cache_is_fresh_only_when_version_and_file_identity_match() {
        let entry = cached(123, 456);
        let v = audio::STRATUM_SCHEMA_VERSION;
        assert!(is_cache_fresh(Some(&entry), v, 123, 456, "hmm:v1"));
        assert!(!is_cache_fresh(
            Some(&entry),
            "outdated",
            123,
            456,
            "hmm:v1"
        ));
        assert!(
            !is_cache_fresh(Some(&entry), v, 999, 456, "hmm:v1"),
            "mismatched file size must be stale"
        );
        assert!(
            !is_cache_fresh(Some(&entry), v, 123, 999, "hmm:v1"),
            "mismatched file mtime must be stale"
        );
    }

    #[test]
    fn missing_cache_is_not_fresh() {
        assert!(!is_cache_fresh(
            None,
            audio::STRATUM_SCHEMA_VERSION,
            123,
            456,
            "hmm:v1"
        ));
    }

    #[test]
    fn file_mtime_unix_returns_non_negative_timestamp_for_real_file() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("x.txt");
        std::fs::write(&path, "a").expect("write");
        let metadata = std::fs::metadata(path).expect("metadata");
        assert!(file_mtime_unix(&metadata) >= 0);
    }

    #[test]
    fn cache_status_skips_track_when_both_fresh_entries_exist() {
        let (_dir, conn, probe) = open_temp_store_with_probe();

        store::set_audio_analysis_with_fingerprint(
            &conn,
            &probe.cache_key,
            "stratum-dsp",
            probe.file_size,
            probe.file_mtime,
            audio::STRATUM_SCHEMA_VERSION,
            &probe.stratum_input_fingerprint,
            "{}",
        )
        .expect("set stratum");
        store::set_audio_analysis_with_fingerprint(
            &conn,
            &probe.cache_key,
            "essentia",
            probe.file_size,
            probe.file_mtime,
            audio::ESSENTIA_SCHEMA_VERSION,
            "",
            "{}",
        )
        .expect("set essentia");

        let (has_stratum, has_essentia) =
            cache_status_for_track(&conn, Some(&probe), true, true).expect("cache status");
        assert!(has_stratum);
        assert!(has_essentia);
    }

    #[test]
    fn cache_status_detects_outdated_schema_version() {
        let (_dir, conn, probe) = open_temp_store_with_probe();

        store::set_audio_analysis_with_fingerprint(
            &conn,
            &probe.cache_key,
            "stratum-dsp",
            probe.file_size,
            probe.file_mtime,
            "outdated",
            &probe.stratum_input_fingerprint,
            "{}",
        )
        .expect("set stale stratum");
        store::set_audio_analysis_with_fingerprint(
            &conn,
            &probe.cache_key,
            "essentia",
            probe.file_size,
            probe.file_mtime,
            audio::ESSENTIA_SCHEMA_VERSION,
            "",
            "{}",
        )
        .expect("set fresh essentia");

        let (has_stratum, has_essentia) =
            cache_status_for_track(&conn, Some(&probe), true, true).expect("cache status");
        assert!(!has_stratum, "outdated stratum cache must be re-analyzed");
        assert!(has_essentia, "fresh essentia cache should still be skipped");
    }

    #[test]
    fn cache_status_detects_stale_file_identity() {
        let (_dir, conn, probe) = open_temp_store_with_probe();

        store::set_audio_analysis_with_fingerprint(
            &conn,
            &probe.cache_key,
            "stratum-dsp",
            probe.file_size + 1,
            probe.file_mtime,
            audio::STRATUM_SCHEMA_VERSION,
            &probe.stratum_input_fingerprint,
            "{}",
        )
        .expect("set stale-size stratum");
        store::set_audio_analysis_with_fingerprint(
            &conn,
            &probe.cache_key,
            "essentia",
            probe.file_size,
            probe.file_mtime + 1,
            audio::ESSENTIA_SCHEMA_VERSION,
            "",
            "{}",
        )
        .expect("set stale-mtime essentia");

        let (has_stratum, has_essentia) =
            cache_status_for_track(&conn, Some(&probe), true, true).expect("cache status");
        assert!(!has_stratum, "stale stratum cache size must be re-analyzed");
        assert!(
            !has_essentia,
            "stale essentia cache mtime must be re-analyzed"
        );
    }

    #[test]
    fn cache_status_grid_change_marks_only_stratum_pending_then_write_restores_freshness() {
        let (_dir, conn, mut probe) = open_temp_store_with_probe();
        store::set_audio_analysis_with_fingerprint(
            &conn,
            &probe.cache_key,
            audio::ANALYZER_STRATUM,
            probe.file_size,
            probe.file_mtime,
            audio::STRATUM_SCHEMA_VERSION,
            "grid:v1:before",
            "{}",
        )
        .unwrap();
        store::set_audio_analysis_with_fingerprint(
            &conn,
            &probe.cache_key,
            audio::ANALYZER_ESSENTIA,
            probe.file_size,
            probe.file_mtime,
            audio::ESSENTIA_SCHEMA_VERSION,
            "",
            "{}",
        )
        .unwrap();
        probe.stratum_input_fingerprint = "grid:v1:after".to_string();

        let (has_stratum, has_essentia) =
            cache_status_for_track(&conn, Some(&probe), true, true).unwrap();
        assert!(!has_stratum);
        assert!(has_essentia);

        persist_cli_cache_message(
            &conn,
            &CliCacheWriteMsg {
                file_path: probe.cache_key.clone(),
                analyzer: audio::ANALYZER_STRATUM.to_string(),
                file_size: probe.file_size,
                file_mtime: probe.file_mtime,
                analyzer_version: audio::STRATUM_SCHEMA_VERSION.to_string(),
                input_fingerprint: probe.stratum_input_fingerprint.clone(),
                features_json: "{}".to_string(),
            },
        )
        .unwrap();

        let (has_stratum, has_essentia) =
            cache_status_for_track(&conn, Some(&probe), true, true).unwrap();
        assert!(has_stratum);
        assert!(has_essentia);
    }

    #[test]
    fn serialize_cache_payload_reports_errors() {
        let err = serialize_cache_payload(&FailingSerialize, "test payload")
            .expect_err("failing serializer should bubble up");
        assert!(err.contains("test payload cache serialization failed"));
        assert!(err.contains("boom"));
    }

    #[tokio::test]
    async fn send_cache_message_reports_closed_channel() {
        tokio::time::timeout(TEST_WATCHDOG, async {
            let (tx, rx) = tokio::sync::mpsc::channel::<CacheWriteRequest<CliCacheWriteMsg>>(1);
            drop(rx);
            let result = bounded(
                send_cache_message(&tx, cache_message(), "analysis"),
                "closed-channel send",
            )
            .await
            .expect("bounded send");
            drop(tx);
            let err = result.expect_err("closed cache channel should surface an error");
            assert!(err.contains("analysis cache queue send failed"));
        })
        .await
        .expect("closed-channel test watchdog expired");
    }

    #[tokio::test]
    async fn send_cache_message_waits_for_success_acknowledgement() {
        tokio::time::timeout(TEST_WATCHDOG, async {
            let (tx, mut rx) = tokio::sync::mpsc::channel::<CacheWriteRequest<CliCacheWriteMsg>>(1);
            let writer = TaskGuard::new(tokio::spawn(async move {
                let request = bounded(rx.recv(), "success writer receive")
                    .await?
                    .ok_or_else(|| "success writer channel closed".to_string())?;
                request.acknowledgement.send(Ok(())).ok();
                Ok::<(), String>(())
            }));

            let send_result = bounded(
                send_cache_message(&tx, cache_message(), "analysis"),
                "success acknowledged send",
            )
            .await;
            drop(tx);
            let writer_result = writer.join("success writer join").await;
            assert_eq!(send_result.expect("bounded send"), Ok(()));
            assert_eq!(writer_result.expect("guarded writer join"), Ok(()));
        })
        .await
        .expect("success-ack test watchdog expired");
    }

    #[tokio::test]
    async fn send_cache_message_reports_write_acknowledgement_error() {
        tokio::time::timeout(TEST_WATCHDOG, async {
            let (tx, mut rx) = tokio::sync::mpsc::channel::<CacheWriteRequest<CliCacheWriteMsg>>(1);
            let writer = TaskGuard::new(tokio::spawn(async move {
                let request = bounded(rx.recv(), "error writer receive")
                    .await?
                    .ok_or_else(|| "error writer channel closed".to_string())?;
                request
                    .acknowledgement
                    .send(Err("sqlite write rejected".to_string()))
                    .ok();
                Ok::<(), String>(())
            }));

            let send_result = bounded(
                send_cache_message(&tx, cache_message(), "analysis"),
                "error acknowledged send",
            )
            .await;
            drop(tx);
            let writer_result = writer.join("error writer join").await;
            let err = send_result
                .expect("bounded send")
                .expect_err("write error must reach producer");
            assert_eq!(err, "sqlite write rejected");
            assert_eq!(writer_result.expect("guarded writer join"), Ok(()));
        })
        .await
        .expect("write-error test watchdog expired");
    }

    #[tokio::test]
    async fn send_cache_message_reports_canceled_acknowledgement() {
        tokio::time::timeout(TEST_WATCHDOG, async {
            let (tx, mut rx) = tokio::sync::mpsc::channel::<CacheWriteRequest<CliCacheWriteMsg>>(1);
            let writer = TaskGuard::new(tokio::spawn(async move {
                let request = bounded(rx.recv(), "canceled writer receive")
                    .await?
                    .ok_or_else(|| "canceled writer channel closed".to_string())?;
                drop(request.acknowledgement);
                Ok::<(), String>(())
            }));

            let send_result = bounded(
                send_cache_message(&tx, cache_message(), "analysis"),
                "canceled acknowledged send",
            )
            .await;
            drop(tx);
            let writer_result = writer.join("canceled writer join").await;
            let err = send_result
                .expect("bounded send")
                .expect_err("dropped acknowledgement must reach producer");
            assert!(err.contains("analysis cache acknowledgement canceled"));
            assert_eq!(writer_result.expect("guarded writer join"), Ok(()));
        })
        .await
        .expect("canceled-ack test watchdog expired");
    }

    #[test]
    fn cli_batch_outcome_accepts_only_complete_success() {
        assert!(cli_batch_outcome("test", 0, 0, 0, 0, false, vec![]).is_ok());
        for (failures, joins, writes, incomplete, cancelled) in [
            (1, 0, 0, 0, false),
            (0, 1, 0, 0, false),
            (0, 0, 1, 0, false),
            (0, 0, 0, 1, false),
            (0, 0, 0, 0, true),
            (1, 1, 1, 1, true),
        ] {
            assert!(
                cli_batch_outcome(
                    "test",
                    failures,
                    joins,
                    writes,
                    incomplete,
                    cancelled,
                    vec!["stable summary".to_string()],
                )
                .is_err()
            );
        }

        let combined =
            cli_batch_outcome("test", 1, 2, 3, 4, true, vec!["stable summary".to_string()])
                .expect_err("combined failures");
        assert_eq!(
            combined,
            super::CliBatchFailure {
                command: "test",
                track_or_provider_failures: 1,
                worker_join_failures: 2,
                writer_failures: 3,
                incomplete: 4,
                user_cancelled: true,
                error_summaries: vec!["stable summary".to_string()],
            }
        );
    }

    #[test]
    fn cli_cancellation_state_distinguishes_user_and_internal_shutdown() {
        let state = CliCancellationState::default();
        let token = tokio_util::sync::CancellationToken::new();
        assert!(!state.user_requested());

        token.cancel();
        assert!(token.is_cancelled());
        assert!(
            !state.user_requested(),
            "internal cancellation is not Ctrl-C"
        );

        state.mark_user_requested();
        assert!(state.user_requested());
    }

    #[test]
    fn cli_cancellation_state_normal_shutdown_is_not_user_cancelled() {
        let state = CliCancellationState::default();
        let token = tokio_util::sync::CancellationToken::new();
        let user_requested_before_shutdown = state.user_requested();
        token.cancel();
        assert!(!user_requested_before_shutdown);
        assert!(!state.user_requested());
    }

    #[tokio::test]
    async fn decode_join_error_marks_failed_and_allows_next_track() {
        tokio::time::timeout(TEST_WATCHDOG, async {
            let handle = TaskGuard::new(tokio::spawn(async {
                tokio::time::sleep(Duration::from_secs(60)).await;
                Ok::<(Vec<f32>, u32), AudioError>((vec![0.0], 44_100))
            }));
            handle.abort();
            let join_err = handle
                .join_raw("aborted decode task join")
                .await
                .expect("bounded decode join")
                .expect_err("aborted task should produce JoinError");

            let mut failed = 0;
            assert!(handle_decode_result(Err(join_err), 1, 2, "a - b", &mut failed).is_none());
            assert_eq!(failed, 1);

            let next =
                handle_decode_result(Ok(Ok((vec![0.0], 44_100))), 2, 2, "c - d", &mut failed);
            assert!(
                next.is_some(),
                "next track should continue after prior join error"
            );
            assert_eq!(failed, 1);
        })
        .await
        .expect("decode-join test watchdog expired");
    }

    #[tokio::test]
    async fn analysis_join_error_marks_failed_and_allows_next_track() {
        tokio::time::timeout(TEST_WATCHDOG, async {
            let handle = TaskGuard::new(tokio::spawn(async {
                tokio::time::sleep(Duration::from_secs(60)).await;
                Ok::<StratumResult, AudioError>(sample_stratum_result())
            }));
            handle.abort();
            let join_err = handle
                .join_raw("aborted analysis task join")
                .await
                .expect("bounded analysis join")
                .expect_err("aborted task should produce JoinError");

            let mut failed = 0;
            assert!(handle_analysis_result(Err(join_err), 1, 2, "a - b", &mut failed).is_none());
            assert_eq!(failed, 1);

            let next =
                handle_analysis_result(Ok(Ok(sample_stratum_result())), 2, 2, "c - d", &mut failed);
            assert!(
                next.is_some(),
                "next track should continue after prior analysis join error"
            );
            assert_eq!(failed, 1);
        })
        .await
        .expect("analysis-join test watchdog expired");
    }

    #[test]
    fn mark_track_outcome_counts_success_and_failure_consistently() {
        let mut analyzed = 0;
        let mut failed = 0;

        mark_track_outcome(&mut analyzed, &mut failed, true);
        mark_track_outcome(&mut analyzed, &mut failed, false);

        assert_eq!(analyzed, 1);
        assert_eq!(failed, 1);
    }

    use super::{
        CpuPreset, MEMORY_MIN_COST_MB, memory_budget_mb, memory_preset_summary,
        system_total_memory_mb, track_memory_cost_mb,
    };

    #[test]
    fn track_memory_cost_zero_duration_returns_minimum() {
        assert_eq!(track_memory_cost_mb(0), MEMORY_MIN_COST_MB);
    }

    #[test]
    fn track_memory_cost_negative_duration_returns_minimum() {
        assert_eq!(track_memory_cost_mb(-30), MEMORY_MIN_COST_MB);
    }

    #[test]
    fn track_memory_cost_six_minute_track() {
        // 6 min * 600 MB/min + 200 MB overhead = 3800 MB
        assert_eq!(track_memory_cost_mb(360), 3800);
    }

    #[test]
    fn track_memory_cost_twenty_minute_track() {
        // 20 min * 600 MB/min + 200 MB overhead = 12200 MB
        assert_eq!(track_memory_cost_mb(1200), 12200);
    }

    #[test]
    fn system_total_memory_is_plausible() {
        let mb = system_total_memory_mb();
        // Any macOS dev machine has at least 4 GB
        assert!(mb >= 4096, "system memory {mb} MB seems too low");
        // Sanity upper bound: 1 TB
        assert!(mb <= 1_048_576, "system memory {mb} MB seems too high");
    }

    #[test]
    fn overnight_budget_exceeds_background_budget() {
        let overnight = memory_budget_mb(CpuPreset::Overnight);
        let background = memory_budget_mb(CpuPreset::Background);
        assert!(
            overnight > background,
            "overnight ({overnight} MB) should exceed background ({background} MB)"
        );
    }

    #[test]
    fn memory_preset_summary_contains_budget() {
        let budget = memory_budget_mb(CpuPreset::Background);
        let summary = memory_preset_summary(budget);
        assert!(summary.contains("Memory:"));
        assert!(summary.contains("GB"));
    }
}
