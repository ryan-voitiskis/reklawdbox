mod analyze;
mod hydrate;
mod setup;
mod tags;

use std::path::{Path, PathBuf};

use clap::Parser;

use crate::{audio, store};

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
}

pub async fn main() -> Result<(), Box<dyn std::error::Error>> {
    eprintln!(
        "{}",
        console::style(format!("reklawdbox v{}", env!("CARGO_PKG_VERSION"))).dim()
    );
    let cli = Cli::parse();
    match cli {
        Cli::Analyze(args) => analyze::run_analyze(args).await,
        Cli::Hydrate(args) => hydrate::run_hydrate(args).await,
        Cli::ReadTags(args) => tags::run_read_tags(args),
        Cli::WriteTags(args) => tags::run_write_tags(args),
        Cli::ExtractArt(args) => tags::run_extract_art(args),
        Cli::EmbedArt(args) => tags::run_embed_art(args),
        Cli::Setup(args) => setup::run_setup(args),
    }
}

fn file_mtime_unix(metadata: &std::fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn is_cache_fresh(cached: Option<&store::CachedAudioAnalysis>, schema_version: &str) -> bool {
    matches!(
        cached,
        Some(entry) if entry.analysis_version == schema_version
    )
}

fn cache_probe_for_path(file_path: &str, skip_cached: bool) -> Option<String> {
    if !skip_cached {
        return None;
    }
    audio::resolve_audio_path(file_path).ok()
}

fn has_fresh_cache_entry(
    store_conn: &rusqlite::Connection,
    cache_key: Option<&str>,
    analyzer: &str,
    schema_version: &str,
) -> Result<bool, rusqlite::Error> {
    if let Some(cache_key) = cache_key {
        let cached = store::get_audio_analysis(store_conn, cache_key, analyzer)?;
        Ok(is_cache_fresh(cached.as_ref(), schema_version))
    } else {
        Ok(false)
    }
}

fn cache_status_for_track(
    store_conn: &rusqlite::Connection,
    cache_key: Option<&str>,
    skip_cached: bool,
    essentia_available: bool,
) -> Result<(bool, bool), rusqlite::Error> {
    let has_stratum = if skip_cached {
        has_fresh_cache_entry(
            store_conn,
            cache_key,
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
            cache_key,
            audio::ANALYZER_ESSENTIA,
            audio::ESSENTIA_SCHEMA_VERSION,
        )?
    } else {
        false
    };

    Ok((has_stratum, has_essentia))
}

pub(crate) struct CliCacheWriteMsg {
    pub file_path: String,
    pub analyzer: String,
    pub file_size: i64,
    pub file_mtime: i64,
    pub analyzer_version: String,
    pub features_json: String,
}

pub(crate) fn serialize_cache_payload<T: serde::Serialize>(
    value: &T,
    context: &str,
) -> Result<String, String> {
    serde_json::to_string(value).map_err(|e| format!("{context} cache serialization failed: {e}"))
}

pub(crate) async fn send_cache_message<T>(
    tx: &tokio::sync::mpsc::Sender<T>,
    message: T,
    context: &str,
) -> Result<(), String> {
    tx.send(message)
        .await
        .map_err(|e| format!("{context} cache queue send failed: {e}"))
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
            for entry in entries.filter_map(|e| e.ok()) {
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
mod tests {
    use super::analyze::{handle_analysis_result, handle_decode_result, mark_track_outcome};
    use super::{
        CliCacheWriteMsg, cache_status_for_track, file_mtime_unix, is_cache_fresh,
        send_cache_message, serialize_cache_payload,
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
            features_json: "{}".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
        }
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
            duration_seconds: 180.0,
            processing_time_ms: 42.0,
            analyzer_version: "1.0.0".to_string(),
            mod_centroid: Some(12.5),
            harmonic_proportion: Some(0.65),
            decay_mid_tau: Some(180.0),
            decay_mid_r2: Some(0.92),
            decay_high_tau: Some(95.0),
            decay_high_r2: Some(0.88),
            flags: vec![],
            warnings: vec![],
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

    fn open_temp_store_with_probe() -> (tempfile::TempDir, rusqlite::Connection, String, i64, i64) {
        let dir = tempfile::tempdir().expect("temp dir");

        let audio_path = dir.path().join("track.wav");
        std::fs::write(&audio_path, b"not-a-real-audio-file").expect("write audio fixture");

        let metadata = std::fs::metadata(&audio_path).expect("metadata");
        let cache_key = audio_path.to_string_lossy().to_string();
        let file_size = metadata.len() as i64;
        let file_mtime = file_mtime_unix(&metadata);

        let store_path = dir.path().join("cache.sqlite3");
        let conn = store::open(store_path.to_str().expect("utf-8 path")).expect("open store");
        (dir, conn, cache_key, file_size, file_mtime)
    }

    #[test]
    fn cache_is_fresh_only_when_version_matches() {
        let entry = cached(123, 456);
        let v = audio::STRATUM_SCHEMA_VERSION;
        assert!(is_cache_fresh(Some(&entry), v));
        assert!(!is_cache_fresh(Some(&entry), "outdated"));
    }

    #[test]
    fn cache_is_fresh_regardless_of_file_metadata_changes() {
        // file_size/file_mtime in the cache entry don't affect freshness —
        // only analysis_version matters. This prevents spurious re-analysis
        // when Rekordbox or other tools write tags to audio files.
        let entry = cached(100, 1000);
        assert!(is_cache_fresh(Some(&entry), audio::STRATUM_SCHEMA_VERSION));
        let entry2 = cached(999, 9999);
        assert!(is_cache_fresh(Some(&entry2), audio::STRATUM_SCHEMA_VERSION));
    }

    #[test]
    fn missing_cache_is_not_fresh() {
        assert!(!is_cache_fresh(None, audio::STRATUM_SCHEMA_VERSION));
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
        let (_dir, conn, cache_key, file_size, file_mtime) = open_temp_store_with_probe();

        store::set_audio_analysis(
            &conn,
            &cache_key,
            "stratum-dsp",
            file_size,
            file_mtime,
            audio::STRATUM_SCHEMA_VERSION,
            "{}",
        )
        .expect("set stratum");
        store::set_audio_analysis(
            &conn,
            &cache_key,
            "essentia",
            file_size,
            file_mtime,
            audio::ESSENTIA_SCHEMA_VERSION,
            "{}",
        )
        .expect("set essentia");

        let (has_stratum, has_essentia) =
            cache_status_for_track(&conn, Some(cache_key.as_str()), true, true)
                .expect("cache status");
        assert!(has_stratum);
        assert!(has_essentia);
    }

    #[test]
    fn cache_status_detects_outdated_schema_version() {
        let (_dir, conn, cache_key, file_size, file_mtime) = open_temp_store_with_probe();

        store::set_audio_analysis(
            &conn,
            &cache_key,
            "stratum-dsp",
            file_size,
            file_mtime,
            "outdated",
            "{}",
        )
        .expect("set stale stratum");
        store::set_audio_analysis(
            &conn,
            &cache_key,
            "essentia",
            file_size,
            file_mtime,
            audio::ESSENTIA_SCHEMA_VERSION,
            "{}",
        )
        .expect("set fresh essentia");

        let (has_stratum, has_essentia) =
            cache_status_for_track(&conn, Some(cache_key.as_str()), true, true)
                .expect("cache status");
        assert!(!has_stratum, "outdated stratum cache must be re-analyzed");
        assert!(has_essentia, "fresh essentia cache should still be skipped");
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
        let (tx, rx) = tokio::sync::mpsc::channel::<CliCacheWriteMsg>(1);
        drop(rx);
        let err = send_cache_message(
            &tx,
            CliCacheWriteMsg {
                file_path: "/tmp/test.flac".to_string(),
                analyzer: "stratum-dsp".to_string(),
                file_size: 1,
                file_mtime: 2,
                analyzer_version: "v1".to_string(),
                features_json: "{}".to_string(),
            },
            "analysis",
        )
        .await
        .expect_err("closed cache channel should surface an error");
        assert!(err.contains("analysis cache queue send failed"));
    }

    #[tokio::test]
    async fn decode_join_error_marks_failed_and_allows_next_track() {
        let handle = tokio::spawn(async {
            tokio::time::sleep(Duration::from_secs(60)).await;
            Ok::<(Vec<f32>, u32), AudioError>((vec![0.0], 44_100))
        });
        handle.abort();
        let join_err = handle
            .await
            .expect_err("aborted task should produce JoinError");

        let mut failed = 0;
        assert!(handle_decode_result(Err(join_err), 1, 2, "a - b", &mut failed).is_none());
        assert_eq!(failed, 1);

        let next = handle_decode_result(Ok(Ok((vec![0.0], 44_100))), 2, 2, "c - d", &mut failed);
        assert!(
            next.is_some(),
            "next track should continue after prior join error"
        );
        assert_eq!(failed, 1);
    }

    #[tokio::test]
    async fn analysis_join_error_marks_failed_and_allows_next_track() {
        let handle = tokio::spawn(async {
            tokio::time::sleep(Duration::from_secs(60)).await;
            Ok::<StratumResult, AudioError>(sample_stratum_result())
        });
        handle.abort();
        let join_err = handle
            .await
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
