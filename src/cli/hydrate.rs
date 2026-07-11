use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use tokio_util::sync::CancellationToken;

use console::style;

use crate::{audio, beatport, db, discogs, normalize, store, tools};

use super::{
    CacheWriteRequest, CacheWriterReport, CliBatchFailure, CliCacheWriteMsg, CliCancellationState,
    cache_probe_for_path, cache_status_for_track, cli_batch_outcome, file_mtime_unix,
    send_cache_message, serialize_cache_payload, task_join_error_summary,
};

#[derive(Clone, Debug, PartialEq)]
enum Provider {
    Discogs,
    Beatport,
    Analysis,
}

#[derive(Clone, Debug)]
struct Providers(Vec<Provider>);

impl Providers {
    fn contains(&self, p: &Provider) -> bool {
        self.0.contains(p)
    }
}

fn parse_providers(s: &str) -> Result<Providers, String> {
    let mut out = Vec::new();
    for part in s.split(',') {
        match part.trim().to_ascii_lowercase().as_str() {
            "discogs" => out.push(Provider::Discogs),
            "beatport" => out.push(Provider::Beatport),
            "analysis" => out.push(Provider::Analysis),
            other => return Err(format!("unknown provider: {other}")),
        }
    }
    if out.is_empty() {
        return Err("no providers specified".into());
    }
    Ok(Providers(out))
}

#[derive(clap::Args)]
pub(crate) struct HydrateArgs {
    /// Providers to run (comma-separated: discogs,beatport,analysis)
    #[arg(long, default_value = "discogs,beatport,analysis", value_parser = parse_providers)]
    providers: Providers,
    /// Filter by playlist ID
    #[arg(long)]
    playlist: Option<String>,
    /// Filter by artist name (partial match)
    #[arg(long)]
    artist: Option<String>,
    /// Filter by genre name (partial match)
    #[arg(long)]
    genre: Option<String>,
    /// Minimum BPM
    #[arg(long)]
    bpm_min: Option<f64>,
    /// Maximum BPM
    #[arg(long)]
    bpm_max: Option<f64>,
    /// Filter by musical key
    #[arg(long)]
    key: Option<String>,
    /// Filter by label name (partial match)
    #[arg(long)]
    label: Option<String>,
    /// Filter by file path/folder (partial match)
    #[arg(long)]
    path: Option<String>,
    /// Search query matching title or artist
    #[arg(long)]
    query: Option<String>,
    /// Only tracks added on or after this date (ISO date)
    #[arg(long)]
    added_after: Option<String>,
    /// Only tracks added on or before this date (ISO date)
    #[arg(long)]
    added_before: Option<String>,
    /// Minimum star rating (1-5)
    #[arg(long)]
    rating_min: Option<u8>,
    /// Max tracks to process (omit for unlimited)
    #[arg(long)]
    max_tracks: Option<u32>,
    /// Don't retry previously-errored enrichments
    #[arg(long)]
    no_retry_errors: bool,
    /// CPU scheduling preset for audio analysis
    #[arg(long, value_enum, default_value_t = super::CpuPreset::Background)]
    cpu: super::CpuPreset,
    /// Enrichment concurrency (default: 4)
    #[arg(long, short = 'j')]
    concurrency: Option<u32>,
    /// Skip confirmation prompt
    #[arg(long, short = 'y')]
    yes: bool,
}

struct ProviderCounters {
    enriched: AtomicU32,
    no_match: AtomicU32,
    errors: AtomicU32,
    // Provider/analysis failures only; cache and task failures have separate reports.
    operation_errors: AtomicU32,
    terminal: AtomicU32,
}

impl ProviderCounters {
    fn new() -> Self {
        Self {
            enriched: AtomicU32::new(0),
            no_match: AtomicU32::new(0),
            errors: AtomicU32::new(0),
            operation_errors: AtomicU32::new(0),
            terminal: AtomicU32::new(0),
        }
    }
}

enum HydrateCacheMsg {
    Enrichment {
        provider: String,
        norm_artist: String,
        norm_title: String,
        norm_album: Option<String>,
        match_quality: Option<String>,
        response_json: Option<String>,
    },
    AudioAnalysis(CliCacheWriteMsg),
}

#[derive(Debug, Default)]
struct HydrateAnalysisOutcome {
    operation_failed: bool,
    cache_write_failed: bool,
}

impl HydrateAnalysisOutcome {
    fn succeeded(&self) -> bool {
        !self.operation_failed && !self.cache_write_failed
    }
}

#[derive(Debug, Default)]
struct ProviderTaskReport {
    selected: usize,
    terminal_workers: usize,
    join_failures: u32,
    error_summaries: Vec<String>,
}

impl ProviderTaskReport {
    fn new(selected: usize) -> Self {
        Self {
            selected,
            ..Self::default()
        }
    }

    fn incomplete(&self) -> usize {
        self.selected.saturating_sub(self.terminal_workers)
    }
}

fn record_provider_worker_join(
    provider: &'static str,
    result: Result<bool, tokio::task::JoinError>,
    counters: &ProviderCounters,
    report: &mut ProviderTaskReport,
) {
    match result {
        Ok(true) => report.terminal_workers += 1,
        Ok(false) => {}
        Err(error) => {
            counters.errors.fetch_add(1, Ordering::Relaxed);
            report.join_failures += 1;
            let summary = task_join_error_summary(&format!("{provider} worker task"), &error);
            tracing::error!("{summary}: {error}");
            report.error_summaries.push(summary);
        }
    }
}

fn resolve_provider_task_join(
    provider: &'static str,
    selected: usize,
    counters: &ProviderCounters,
    result: Result<ProviderTaskReport, tokio::task::JoinError>,
) -> ProviderTaskReport {
    match result {
        Ok(report) => report,
        Err(error) => {
            counters.errors.fetch_add(1, Ordering::Relaxed);
            let summary = task_join_error_summary(&format!("{provider} provider task"), &error);
            tracing::error!("{summary}: {error}");
            ProviderTaskReport {
                selected,
                terminal_workers: counters.terminal.load(Ordering::Relaxed) as usize,
                join_failures: 1,
                error_summaries: vec![summary],
            }
        }
    }
}

fn run_hydrate_cache_writer(
    store_path: String,
    mut cache_rx: tokio::sync::mpsc::Receiver<CacheWriteRequest<HydrateCacheMsg>>,
    cancel: CancellationToken,
) -> CacheWriterReport {
    let mut report = CacheWriterReport::default();
    let conn = match store::open(&store_path) {
        Ok(conn) => conn,
        Err(error) => {
            let summary = format!("cache store open failed: {error}");
            tracing::error!("Cache writer: {summary} — rejecting queued writes");
            report.error_summaries.push(summary.clone());
            cancel.cancel();
            while let Some(request) = cache_rx.blocking_recv() {
                report.record_failure(summary.clone());
                request.acknowledgement.send(Err(summary.clone())).ok();
            }
            return report;
        }
    };

    let mut consecutive_failures: u32 = 0;
    let mut fatal_error: Option<String> = None;
    while let Some(request) = cache_rx.blocking_recv() {
        if let Some(summary) = &fatal_error {
            report.record_failure(summary.clone());
            request.acknowledgement.send(Err(summary.clone())).ok();
            continue;
        }

        let message = request.payload;
        let (write_result, label) = match &message {
            HydrateCacheMsg::Enrichment {
                provider,
                norm_artist,
                norm_title,
                norm_album,
                match_quality,
                response_json,
            } => (
                store::set_enrichment(
                    &conn,
                    provider,
                    norm_artist,
                    norm_title,
                    norm_album.as_deref(),
                    match_quality.as_deref(),
                    response_json.as_deref(),
                ),
                format!("{provider} enrichment"),
            ),
            HydrateCacheMsg::AudioAnalysis(analysis) => (
                super::persist_cli_cache_message(&conn, analysis),
                format!("{} analysis", analysis.analyzer),
            ),
        };

        match write_result {
            Ok(()) => {
                consecutive_failures = 0;
                report.record_success();
                request.acknowledgement.send(Ok(())).ok();
            }
            Err(error) => {
                consecutive_failures += 1;
                let summary = format!("{label} cache write failed: {error}");
                tracing::error!(
                    "Cache writer: {summary} ({consecutive_failures}/{})",
                    super::MAX_CONSECUTIVE_CACHE_WRITE_FAILURES,
                );
                report.record_failure(summary.clone());
                request.acknowledgement.send(Err(summary)).ok();
                if consecutive_failures >= super::MAX_CONSECUTIVE_CACHE_WRITE_FAILURES {
                    let fatal = format!(
                        "cache writer stopped after {} consecutive failures",
                        super::MAX_CONSECUTIVE_CACHE_WRITE_FAILURES
                    );
                    tracing::error!("Cache writer: {fatal} — draining queued writes");
                    report.threshold_cancelled = true;
                    if report.error_summaries.len() < 10 && !report.error_summaries.contains(&fatal)
                    {
                        report.error_summaries.push(fatal.clone());
                    }
                    fatal_error = Some(fatal);
                    cancel.cancel();
                }
            }
        }
    }
    report
}

fn hydrate_batch_outcome(
    provider_failures: u32,
    join_failures: u32,
    writer_failures: u32,
    incomplete: usize,
    user_cancelled: bool,
    error_summaries: Vec<String>,
) -> Result<(), CliBatchFailure> {
    cli_batch_outcome(
        "hydrate",
        provider_failures,
        join_failures,
        writer_failures,
        incomplete,
        user_cancelled,
        error_summaries,
    )
}

pub(crate) async fn run_hydrate(args: HydrateArgs) -> Result<(), Box<dyn std::error::Error>> {
    let cpu_preset = args.cpu;
    super::apply_cpu_niceness(cpu_preset);
    let analysis_concurrency = super::analysis_concurrency_for_preset(cpu_preset);
    let analysis_budget_mb = super::memory_budget_mb(cpu_preset);

    let want_discogs = args.providers.contains(&Provider::Discogs);
    let want_beatport = args.providers.contains(&Provider::Beatport);
    let want_analysis = args.providers.contains(&Provider::Analysis);

    // 1. Bootstrap
    let db_path = db::resolve_db_path().ok_or(
        "Cannot find Rekordbox database. Set REKORDBOX_DB_PATH or ensure Rekordbox is installed.",
    )?;
    let conn = db::open(&db_path)?;
    let store_path = store::default_path();
    let store_path_str = store_path
        .to_str()
        .ok_or("Invalid store path encoding")?
        .to_string();
    let store_conn = store::open(&store_path_str)?;
    let client = reqwest::Client::builder()
        .user_agent("Reklawdbox/0.1")
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .expect("failed to build HTTP client");

    let essentia_python = if want_analysis {
        tools::probe_essentia_python_path()
    } else {
        None
    };

    // 2. Resolve tracks
    let params = db::SearchParams {
        query: args.query,
        artist: args.artist,
        genre: args.genre,
        rating_min: args.rating_min,
        bpm_min: args.bpm_min,
        bpm_max: args.bpm_max,
        key: args.key,
        playlist: args.playlist,
        has_genre: None,
        has_label: None,
        year_zero: None,
        label: args.label,
        path: args.path,
        path_prefix: None,
        added_after: args
            .added_after
            .map(|s| db::validate_iso_date(&s, "added_after"))
            .transpose()?,
        added_before: args
            .added_before
            .map(|s| db::validate_iso_date(&s, "added_before"))
            .transpose()?,
        exclude_samples: true,
        limit: args.max_tracks,
        offset: None,
    };
    let tracks = db::search_tracks_unbounded(&conn, &params)?;
    drop(conn);

    if tracks.is_empty() {
        println!("No tracks match the given filters.");
        return Ok(());
    }

    // 3. Pre-filter by cache per provider
    let retry_errors = !args.no_retry_errors;
    let mut discogs_pending = Vec::new();
    let mut discogs_cached: u32 = 0;
    let mut discogs_errors: u32 = 0;
    let mut beatport_pending = Vec::new();
    let mut beatport_cached: u32 = 0;
    let mut beatport_errors: u32 = 0;
    let mut analysis_pending = Vec::new();
    let mut analysis_cached: u32 = 0;

    for track in &tracks {
        let norm_artist = normalize::normalize_for_matching(&track.artist);
        let norm_title = normalize::normalize_for_matching(&track.title);
        let norm_album = normalize::normalize_for_matching(&track.album);
        let norm_album = (!norm_album.is_empty()).then_some(norm_album);

        if want_discogs {
            match store::get_enrichment(
                &store_conn,
                "discogs",
                &norm_artist,
                &norm_title,
                norm_album.as_deref(),
                true,
            )? {
                Some(entry) => {
                    if entry.match_quality.as_deref() == Some("error") {
                        discogs_errors += 1;
                        if retry_errors {
                            discogs_pending.push(track.clone());
                        }
                    } else {
                        discogs_cached += 1;
                    }
                }
                None => {
                    discogs_pending.push(track.clone());
                }
            }
        }

        if want_beatport {
            match store::get_enrichment(
                &store_conn,
                "beatport",
                &norm_artist,
                &norm_title,
                None,
                true,
            )? {
                Some(entry) => {
                    if entry.match_quality.as_deref() == Some("error") {
                        beatport_errors += 1;
                        if retry_errors {
                            beatport_pending.push(track.clone());
                        }
                    } else {
                        beatport_cached += 1;
                    }
                }
                None => {
                    beatport_pending.push(track.clone());
                }
            }
        }

        if want_analysis {
            let cache_probe = cache_probe_for_path(&track.file_path, true);
            let (has_stratum, has_essentia) = cache_status_for_track(
                &store_conn,
                cache_probe.as_ref(),
                true,
                essentia_python.is_some(),
            )?;
            if has_stratum && has_essentia {
                analysis_cached += 1;
            } else {
                analysis_pending.push((track.clone(), !has_stratum, !has_essentia));
            }
        }
    }

    // LPT scheduling: longest tracks first so short tracks fill gaps at the tail
    analysis_pending.sort_by_key(|b| std::cmp::Reverse(b.0.length));

    drop(store_conn);

    let total_tracks = tracks.len();
    let discogs_selected = discogs_pending.len();
    let beatport_selected = beatport_pending.len();
    let analysis_selected = analysis_pending.len();
    let total_work = discogs_selected + beatport_selected + analysis_selected;

    if total_work == 0 {
        println!("Found {total_tracks} tracks matching filters.");
        println!("All cached. Nothing to do.");
        return Ok(());
    }

    // 4. Startup summary
    println!("Found {total_tracks} tracks matching filters.");
    println!(
        "  {}",
        super::cpu_preset_summary(cpu_preset, analysis_concurrency)
    );
    if want_analysis {
        println!("  {}", super::memory_preset_summary(analysis_budget_mb));
    }
    if want_discogs {
        let retry_note = if discogs_errors > 0 && retry_errors {
            format!(", {discogs_errors} errors to retry")
        } else if discogs_errors > 0 {
            format!(", {discogs_errors} errors (skipped)")
        } else {
            String::new()
        };
        println!(
            "  Discogs:  {} cached{}, {} pending",
            discogs_cached,
            retry_note,
            discogs_pending.len()
        );
    }
    if want_beatport {
        let retry_note = if beatport_errors > 0 && retry_errors {
            format!(", {beatport_errors} errors to retry")
        } else if beatport_errors > 0 {
            format!(", {beatport_errors} errors (skipped)")
        } else {
            String::new()
        };
        println!(
            "  Beatport: {} cached{}, {} pending",
            beatport_cached,
            retry_note,
            beatport_pending.len()
        );
    }
    if want_analysis {
        let essentia_note = match &essentia_python {
            Some(_) => "",
            None => " (stratum-dsp only)",
        };
        println!(
            "  Analysis: {} cached, {} pending{}",
            analysis_cached,
            analysis_pending.len(),
            essentia_note,
        );
    }

    let beatport_secs = beatport_pending.len() as u64; // ~1 req/s rate limit
    let discogs_secs = discogs_pending.len() as u64; // ~1 req/1.1s rate limit (serialized by client)
    // stratum-dsp ≈ 18s/track, essentia subprocess adds ≈ 30s/track
    let secs_per_analysis: u64 = if essentia_python.is_some() { 48 } else { 18 };
    // Effective concurrency is min(CPU semaphore, memory semaphore). The memory
    // semaphore is often tighter — e.g. a 6-min track costs ~3.8 GB, so a 7.2 GB
    // background budget only fits ~1-2 concurrent despite 4+ CPU slots.
    let effective_analysis_concurrency = if analysis_pending.is_empty() {
        analysis_concurrency
    } else {
        let avg_cost_mb = analysis_pending
            .iter()
            .map(|(track, _, _)| {
                super::track_memory_cost_mb(track.length).min(analysis_budget_mb) as u64
            })
            .sum::<u64>()
            / analysis_pending.len() as u64;
        let mem_concurrency = (analysis_budget_mb as u64 / avg_cost_mb.max(1)) as usize;
        analysis_concurrency.min(mem_concurrency).max(1)
    };
    let analysis_secs = (analysis_pending.len() as u64 * secs_per_analysis)
        / effective_analysis_concurrency.max(1) as u64;
    let estimated_secs = beatport_secs.max(discogs_secs).max(analysis_secs);
    if estimated_secs > 60 {
        let hours = estimated_secs / 3600;
        let mins = (estimated_secs % 3600) / 60;
        if hours > 0 {
            println!("\nEstimated time: ~{hours}h{mins:02}m");
        } else {
            println!("\nEstimated time: ~{mins}m");
        }
    }

    // 5. Confirmation
    if !args.yes {
        print!("Continue? [Y/n] ");
        use std::io::Write;
        std::io::stdout().flush()?;
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        let trimmed = input.trim().to_ascii_lowercase();
        if !trimmed.is_empty() && trimmed != "y" && trimmed != "yes" {
            println!("Aborted.");
            return Ok(());
        }
    }

    // 6. Discogs auth (if needed)
    let discogs_mode = if want_discogs && !discogs_pending.is_empty() {
        Some(cli_ensure_discogs_auth(&client, &store_path_str).await?)
    } else {
        None
    };

    // 7. Setup cancellation + progress
    let cancel = CancellationToken::new();
    let mp = MultiProgress::new();

    let pb = mp.add(ProgressBar::new(total_work as u64));
    pb.set_style(
        ProgressStyle::with_template(
            "Hydrating [{bar:40.cyan/blue}] {pos}/{len}  {percent}%  ETA {eta}",
        )
        .unwrap()
        .progress_chars("##-"),
    );

    let status_pb = mp.add(ProgressBar::new_spinner());
    status_pb.set_style(ProgressStyle::with_template("  {msg}").unwrap());
    status_pb.enable_steady_tick(Duration::from_secs(1));

    let cancellation_state = CliCancellationState::default();
    super::spawn_signal_handlers(&mp, &cancel, &cancellation_state);

    let discogs_counters = Arc::new(ProviderCounters::new());
    let beatport_counters = Arc::new(ProviderCounters::new());
    let analysis_counters = Arc::new(ProviderCounters::new());

    // 8. Cache writer task
    let enrich_concurrency = args.concurrency.unwrap_or(4).clamp(1, 16) as usize;
    let (cache_tx, cache_rx) = tokio::sync::mpsc::channel::<CacheWriteRequest<HydrateCacheMsg>>(
        enrich_concurrency * 8 + 32,
    );

    let writer_store_path = store_path_str.clone();
    let writer_cancel = cancel.clone();
    let writer_handle = tokio::task::spawn_blocking(move || {
        run_hydrate_cache_writer(writer_store_path, cache_rx, writer_cancel)
    });

    // 9. Spawn provider loops concurrently (each in its own task so serial
    //    Beatport doesn't block analysis spawning)
    let dc = discogs_counters.clone();
    let bc = beatport_counters.clone();
    let ac = analysis_counters.clone();
    let status_cancel = cancel.clone();
    let status_pb_clone = status_pb.clone();
    let want_d = want_discogs;
    let want_b = want_beatport;
    let want_a = want_analysis;
    let status_task = tokio::spawn(async move {
        loop {
            if status_cancel.is_cancelled() {
                break;
            }
            let mut parts = Vec::new();
            if want_d {
                parts.push(format!(
                    "Discogs: {} enriched, {} errors",
                    dc.enriched.load(Ordering::Relaxed),
                    dc.errors.load(Ordering::Relaxed),
                ));
            }
            if want_b {
                parts.push(format!(
                    "Beatport: {} enriched, {} errors",
                    bc.enriched.load(Ordering::Relaxed),
                    bc.errors.load(Ordering::Relaxed),
                ));
            }
            if want_a {
                parts.push(format!(
                    "Analysis: {} done, {} errors",
                    ac.enriched.load(Ordering::Relaxed),
                    ac.errors.load(Ordering::Relaxed),
                ));
            }
            status_pb_clone.set_message(parts.join(" | "));
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    });

    let batch_start = Instant::now();

    let discogs_task = {
        let cancel = cancel.clone();
        let client = client.clone();
        let cache_tx = cache_tx.clone();
        let counters = discogs_counters.clone();
        let pb = pb.clone();
        tokio::spawn(async move {
            let mut report = ProviderTaskReport::new(discogs_selected);
            if discogs_pending.is_empty() {
                return report;
            }
            let (broker_cfg, session_token) = match discogs_mode {
                Some(m) => m,
                None => return report,
            };
            let sem = Arc::new(tokio::sync::Semaphore::new(enrich_concurrency));
            let mut handles = tokio::task::JoinSet::new();

            for track in discogs_pending {
                if cancel.is_cancelled() {
                    break;
                }
                let permit = match sem.clone().acquire_owned().await {
                    Ok(p) => p,
                    Err(_) => break,
                };
                let client = client.clone();
                let cache_tx = cache_tx.clone();
                let counters = counters.clone();
                let pb = pb.clone();
                let cfg = broker_cfg.clone();
                let token = session_token.clone();
                let cancel = cancel.clone();

                handles.spawn(async move {
                    if cancel.is_cancelled() {
                        drop(permit);
                        return false;
                    }

                    let norm_artist = normalize::normalize_for_matching(&track.artist);
                    let norm_title = normalize::normalize_for_matching(&track.title);
                    let norm_album = normalize::normalize_for_matching(&track.album);
                    let norm_album = (!norm_album.is_empty()).then_some(norm_album);

                    let result = cli_discogs_lookup_with_retry(
                        &client,
                        &cfg,
                        &token,
                        &track.artist,
                        &track.title,
                        Some(&track.album),
                    )
                    .await
                    .map_err(|e| e.to_string());

                    match result {
                        Ok(Some(ref r)) => {
                            let quality = if r.fuzzy_match { "fuzzy" } else { "exact" };
                            match serialize_cache_payload(r, "discogs enrichment") {
                                Ok(response_json) => {
                                    if let Err(e) = send_cache_message(
                                        &cache_tx,
                                        HydrateCacheMsg::Enrichment {
                                            provider: "discogs".to_string(),
                                            norm_artist,
                                            norm_title,
                                            norm_album,
                                            match_quality: Some(quality.to_string()),
                                            response_json: Some(response_json),
                                        },
                                        "discogs enrichment",
                                    )
                                    .await
                                    {
                                        tracing::error!("{e}");
                                        counters.errors.fetch_add(1, Ordering::Relaxed);
                                    } else {
                                        counters.enriched.fetch_add(1, Ordering::Relaxed);
                                    }
                                }
                                Err(e) => {
                                    tracing::error!("{e}");
                                    counters.errors.fetch_add(1, Ordering::Relaxed);
                                    counters.operation_errors.fetch_add(1, Ordering::Relaxed);
                                }
                            }
                        }
                        Ok(None) => {
                            if let Err(e) = send_cache_message(
                                &cache_tx,
                                HydrateCacheMsg::Enrichment {
                                    provider: "discogs".to_string(),
                                    norm_artist,
                                    norm_title,
                                    norm_album,
                                    match_quality: Some("none".to_string()),
                                    response_json: None,
                                },
                                "discogs enrichment",
                            )
                            .await
                            {
                                tracing::error!("{e}");
                                counters.errors.fetch_add(1, Ordering::Relaxed);
                            } else {
                                counters.no_match.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        Err(err) => {
                            tracing::error!(
                                "Discogs hydrate lookup failed for {} - {}: {err}",
                                track.artist,
                                track.title
                            );
                            if let Err(error) = send_cache_message(
                                &cache_tx,
                                HydrateCacheMsg::Enrichment {
                                    provider: "discogs".to_string(),
                                    norm_artist,
                                    norm_title,
                                    norm_album,
                                    match_quality: Some("error".to_string()),
                                    response_json: None,
                                },
                                "discogs error",
                            )
                            .await
                            {
                                tracing::error!("{error}");
                            }
                            counters.errors.fetch_add(1, Ordering::Relaxed);
                            counters.operation_errors.fetch_add(1, Ordering::Relaxed);
                        }
                    }

                    counters.terminal.fetch_add(1, Ordering::Relaxed);
                    pb.inc(1);
                    drop(permit);
                    true
                });
            }

            while let Some(result) = handles.join_next().await {
                record_provider_worker_join("discogs", result, &counters, &mut report);
            }
            report
        })
    };

    // Beatport is serial (semaphore=1) due to rate limits
    let beatport_task = {
        let cancel = cancel.clone();
        let client = client.clone();
        let cache_tx = cache_tx.clone();
        let counters = beatport_counters.clone();
        let pb = pb.clone();
        tokio::spawn(async move {
            let mut report = ProviderTaskReport::new(beatport_selected);
            if beatport_pending.is_empty() {
                return report;
            }
            let sem = Arc::new(tokio::sync::Semaphore::new(1));
            let mut handles = tokio::task::JoinSet::new();

            for track in beatport_pending {
                if cancel.is_cancelled() {
                    break;
                }
                let permit = match sem.clone().acquire_owned().await {
                    Ok(p) => p,
                    Err(_) => break,
                };
                let client = client.clone();
                let cache_tx = cache_tx.clone();
                let counters = counters.clone();
                let pb = pb.clone();
                let cancel = cancel.clone();

                handles.spawn(async move {
                    if cancel.is_cancelled() {
                        drop(permit);
                        return false;
                    }

                    let norm_artist = normalize::normalize_for_matching(&track.artist);
                    let norm_title = normalize::normalize_for_matching(&track.title);

                    let result =
                        cli_beatport_lookup_with_retry(&client, &track.artist, &track.title).await;

                    match result {
                        Ok(Some(ref r)) => {
                            match serialize_cache_payload(r, "beatport enrichment") {
                                Ok(response_json) => {
                                    if let Err(e) = send_cache_message(
                                        &cache_tx,
                                        HydrateCacheMsg::Enrichment {
                                            provider: "beatport".to_string(),
                                            norm_artist,
                                            norm_title,
                                            norm_album: None,
                                            match_quality: Some(
                                                if r.fuzzy_match { "fuzzy" } else { "exact" }
                                                    .to_string(),
                                            ),
                                            response_json: Some(response_json),
                                        },
                                        "beatport enrichment",
                                    )
                                    .await
                                    {
                                        tracing::error!("{e}");
                                        counters.errors.fetch_add(1, Ordering::Relaxed);
                                    } else {
                                        counters.enriched.fetch_add(1, Ordering::Relaxed);
                                    }
                                }
                                Err(e) => {
                                    tracing::error!("{e}");
                                    counters.errors.fetch_add(1, Ordering::Relaxed);
                                    counters.operation_errors.fetch_add(1, Ordering::Relaxed);
                                }
                            }
                        }
                        Ok(None) => {
                            if let Err(e) = send_cache_message(
                                &cache_tx,
                                HydrateCacheMsg::Enrichment {
                                    provider: "beatport".to_string(),
                                    norm_artist,
                                    norm_title,
                                    norm_album: None,
                                    match_quality: Some("none".to_string()),
                                    response_json: None,
                                },
                                "beatport enrichment",
                            )
                            .await
                            {
                                tracing::error!("{e}");
                                counters.errors.fetch_add(1, Ordering::Relaxed);
                            } else {
                                counters.no_match.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        Err(err) => {
                            tracing::error!(
                                "Beatport hydrate lookup failed for {} - {}: {err}",
                                track.artist,
                                track.title
                            );
                            if let Err(error) = send_cache_message(
                                &cache_tx,
                                HydrateCacheMsg::Enrichment {
                                    provider: "beatport".to_string(),
                                    norm_artist,
                                    norm_title,
                                    norm_album: None,
                                    match_quality: Some("error".to_string()),
                                    response_json: None,
                                },
                                "beatport error",
                            )
                            .await
                            {
                                tracing::error!("{error}");
                            }
                            counters.errors.fetch_add(1, Ordering::Relaxed);
                            counters.operation_errors.fetch_add(1, Ordering::Relaxed);
                        }
                    }

                    counters.terminal.fetch_add(1, Ordering::Relaxed);
                    pb.inc(1);
                    drop(permit);
                    true
                });
            }

            while let Some(result) = handles.join_next().await {
                record_provider_worker_join("beatport", result, &counters, &mut report);
            }
            report
        })
    };

    let analysis_task = {
        let cancel = cancel.clone();
        let cache_tx = cache_tx.clone();
        let counters = analysis_counters.clone();
        let pb = pb.clone();
        tokio::spawn(async move {
            let mut report = ProviderTaskReport::new(analysis_selected);
            if analysis_pending.is_empty() {
                return report;
            }
            let cpu_sem = Arc::new(tokio::sync::Semaphore::new(analysis_concurrency));
            let mem_sem = Arc::new(tokio::sync::Semaphore::new(analysis_budget_mb as usize));
            let mut handles = tokio::task::JoinSet::new();

            for (track, needs_stratum, needs_essentia) in analysis_pending {
                if cancel.is_cancelled() {
                    break;
                }
                let cost_mb = super::track_memory_cost_mb(track.length).min(analysis_budget_mb);
                let cpu_permit = tokio::select! {
                    result = cpu_sem.clone().acquire_owned() => match result {
                        Ok(p) => p,
                        Err(_) => break,
                    },
                    _ = cancel.cancelled() => break,
                };
                let mem_permit = tokio::select! {
                    result = mem_sem.clone().acquire_many_owned(cost_mb) => match result {
                        Ok(p) => p,
                        Err(_) => break,
                    },
                    _ = cancel.cancelled() => {
                        drop(cpu_permit);
                        break;
                    },
                };
                let essentia_python = essentia_python.clone();
                let cache_tx = cache_tx.clone();
                let counters = counters.clone();
                let pb = pb.clone();
                let cancel = cancel.clone();

                handles.spawn(async move {
                    if cancel.is_cancelled() {
                        drop(cpu_permit);
                        drop(mem_permit);
                        return false;
                    }

                    let outcome = cli_analyze_for_hydrate(
                        &track.file_path,
                        needs_stratum,
                        needs_essentia,
                        essentia_python.as_deref(),
                        &cache_tx,
                    )
                    .await;

                    if outcome.succeeded() {
                        counters.enriched.fetch_add(1, Ordering::Relaxed);
                    } else {
                        counters.errors.fetch_add(1, Ordering::Relaxed);
                    }
                    if outcome.operation_failed {
                        counters.operation_errors.fetch_add(1, Ordering::Relaxed);
                    }

                    counters.terminal.fetch_add(1, Ordering::Relaxed);
                    pb.inc(1);
                    drop(cpu_permit);
                    drop(mem_permit);
                    true
                });
            }

            while let Some(result) = handles.join_next().await {
                record_provider_worker_join("analysis", result, &counters, &mut report);
            }
            report
        })
    };

    // Drop our sender so the writer sees EOF when all producer tasks finish
    drop(cache_tx);

    let discogs_report = resolve_provider_task_join(
        "discogs",
        discogs_selected,
        &discogs_counters,
        discogs_task.await,
    );
    let beatport_report = resolve_provider_task_join(
        "beatport",
        beatport_selected,
        &beatport_counters,
        beatport_task.await,
    );
    let analysis_report = resolve_provider_task_join(
        "analysis",
        analysis_selected,
        &analysis_counters,
        analysis_task.await,
    );

    let user_cancelled = cancellation_state.user_requested();
    cancel.cancel();
    let mut error_summaries = Vec::new();
    error_summaries.extend(discogs_report.error_summaries.iter().cloned());
    error_summaries.extend(beatport_report.error_summaries.iter().cloned());
    error_summaries.extend(analysis_report.error_summaries.iter().cloned());

    let status_join_failures = match status_task.await {
        Ok(()) => 0_u32,
        Err(error) => {
            let summary = task_join_error_summary("hydrate status task", &error);
            tracing::error!("{summary}: {error}");
            error_summaries.push(summary);
            1
        }
    };
    let (writer_report, writer_join_failures) = match writer_handle.await {
        Ok(report) => (report, 0_u32),
        Err(error) => {
            let summary = task_join_error_summary("hydrate cache writer task", &error);
            tracing::error!("{summary}: {error}");
            error_summaries.push(summary);
            (CacheWriterReport::default(), 1_u32)
        }
    };
    error_summaries.extend(writer_report.error_summaries.iter().cloned());
    status_pb.finish_and_clear();
    pb.finish_and_clear();
    mp.clear().ok();

    // 10. Summary
    let elapsed = batch_start.elapsed();
    let mins = elapsed.as_secs() / 60;
    let secs = elapsed.as_secs() % 60;

    println!("\nDone ({mins}m {secs}s)");
    if want_discogs {
        let enriched = discogs_counters.enriched.load(Ordering::Relaxed);
        let no_match = discogs_counters.no_match.load(Ordering::Relaxed);
        let errors = discogs_counters.errors.load(Ordering::Relaxed);
        println!(
            "  Discogs:  {} enriched, {} no match, {} errors",
            style(enriched).green(),
            style(no_match).dim(),
            if errors > 0 {
                style(errors).red()
            } else {
                style(errors).dim()
            },
        );
    }
    if want_beatport {
        let enriched = beatport_counters.enriched.load(Ordering::Relaxed);
        let no_match = beatport_counters.no_match.load(Ordering::Relaxed);
        let errors = beatport_counters.errors.load(Ordering::Relaxed);
        println!(
            "  Beatport: {} enriched, {} no match, {} errors",
            style(enriched).green(),
            style(no_match).dim(),
            if errors > 0 {
                style(errors).red()
            } else {
                style(errors).dim()
            },
        );
    }
    if want_analysis {
        let done = analysis_counters.enriched.load(Ordering::Relaxed);
        let errors = analysis_counters.errors.load(Ordering::Relaxed);
        println!(
            "  Analysis: {} done, {} errors",
            style(done).green(),
            if errors > 0 {
                style(errors).red()
            } else {
                style(errors).dim()
            },
        );
    }

    if writer_report.attempted > 0 || writer_report.failed > 0 {
        println!(
            "  Cache writes: {} succeeded, {} failed",
            style(writer_report.succeeded).green(),
            if writer_report.failed > 0 {
                style(writer_report.failed).red()
            } else {
                style(writer_report.failed).dim()
            }
        );
    }
    if writer_join_failures > 0 {
        println!("  Cache writer task: {}", style("failed").red());
    }

    let incomplete =
        discogs_report.incomplete() + beatport_report.incomplete() + analysis_report.incomplete();
    if incomplete > 0 {
        println!("  Incomplete: {} selected tasks", style(incomplete).red());
    }
    if user_cancelled {
        println!("Cancelled");
    }

    let provider_failures = discogs_counters.operation_errors.load(Ordering::Relaxed)
        + beatport_counters.operation_errors.load(Ordering::Relaxed)
        + analysis_counters.operation_errors.load(Ordering::Relaxed);
    let provider_join_failures = discogs_report.join_failures
        + beatport_report.join_failures
        + analysis_report.join_failures;

    hydrate_batch_outcome(
        provider_failures,
        provider_join_failures + status_join_failures + writer_join_failures,
        writer_report.failed,
        incomplete,
        user_cancelled,
        error_summaries,
    )
    .map_err(|error| Box::new(error) as Box<dyn std::error::Error>)
}

async fn cli_ensure_discogs_auth(
    client: &reqwest::Client,
    store_path: &str,
) -> Result<(discogs::BrokerConfig, String), Box<dyn std::error::Error>> {
    match discogs::BrokerConfig::from_env() {
        discogs::BrokerConfigStatus::Ok(cfg) => {
            let store_conn = store::open(store_path)?;
            if let Some(session) = store::get_broker_discogs_session(&store_conn, &cfg.base_url)? {
                let now = chrono::Utc::now().timestamp();
                if session.expires_at > now {
                    println!("Discogs: using existing broker session");
                    if session.expires_at - now < 3600 {
                        println!("  Warning: session expires in <1 hour");
                    }
                    return Ok((cfg, session.session_token));
                }
                // Expired — clear and re-auth
                store::clear_broker_discogs_session(&store_conn, &cfg.base_url)?;
            }
            drop(store_conn);

            println!("Discogs: starting broker authentication...");
            let pending = discogs::device_session_start(client, &cfg)
                .await
                .map_err(|e| format!("Failed to start Discogs auth: {e}"))?;

            println!("Please authorize at: {}", pending.auth_url);
            let _ = std::process::Command::new("open")
                .arg(&pending.auth_url)
                .spawn();

            let spinner = ProgressBar::new_spinner();
            spinner.set_style(
                ProgressStyle::with_template("{spinner:.green} Waiting for authorization...")
                    .unwrap(),
            );
            spinner.enable_steady_tick(Duration::from_millis(200));

            let poll_interval = Duration::from_secs(pending.poll_interval_seconds.max(2) as u64);

            loop {
                tokio::time::sleep(poll_interval).await;

                let now = chrono::Utc::now().timestamp();
                if now >= pending.expires_at {
                    spinner.finish_and_clear();
                    return Err("Discogs device auth session expired. Please retry.".into());
                }

                let status = discogs::device_session_status(client, &cfg, &pending)
                    .await
                    .map_err(|e| format!("Auth poll failed: {e}"))?;

                match status.status.as_str() {
                    "authorized" | "finalized" => {
                        let finalized = discogs::device_session_finalize(client, &cfg, &pending)
                            .await
                            .map_err(|e| format!("Auth finalize failed: {e}"))?;

                        let store_conn = store::open(store_path)?;
                        store::set_broker_discogs_session(
                            &store_conn,
                            &cfg.base_url,
                            &finalized.session_token,
                            finalized.expires_at,
                        )?;

                        spinner.finish_and_clear();
                        println!("Discogs: authenticated successfully");
                        return Ok((cfg, finalized.session_token));
                    }
                    "pending" => continue,
                    other => {
                        spinner.finish_and_clear();
                        return Err(format!("Unexpected auth status: {other}").into());
                    }
                }
            }
        }
        discogs::BrokerConfigStatus::InvalidUrl(url) => {
            Err(format!("Invalid Discogs broker URL: {url}").into())
        }
    }
}

async fn cli_discogs_lookup_with_retry(
    client: &reqwest::Client,
    cfg: &discogs::BrokerConfig,
    token: &str,
    artist: &str,
    title: &str,
    album: Option<&str>,
) -> Result<Option<discogs::DiscogsResult>, discogs::LookupError> {
    const MAX_ATTEMPTS: u32 = 4;

    for attempt in 0..MAX_ATTEMPTS {
        match discogs::lookup_via_broker(client, cfg, token, artist, title, album).await {
            Ok(result) => return Ok(result),
            Err(e) => {
                // Defence-in-depth: the broker handles Discogs 429s internally,
                // but platform-level rate limits (Cloudflare) or custom brokers
                // may 429.
                let backoff = match &e {
                    discogs::LookupError::Http {
                        status: 429,
                        retry_after,
                        body,
                        ..
                    } => {
                        let wait = retry_after
                            .as_deref()
                            .and_then(|s| s.parse::<u64>().ok())
                            .unwrap_or(5)
                            .min(120);
                        tracing::warn!(status = 429, attempt, wait, "Discogs broker 429: {body}");
                        Some(wait)
                    }
                    discogs::LookupError::Http { status, body, .. }
                        if (500..=599).contains(status) =>
                    {
                        let wait = 5 * 2u64.pow(attempt);
                        tracing::warn!(status, attempt, wait, "Discogs broker {status}: {body}");
                        Some(wait)
                    }
                    discogs::LookupError::Message(msg) => {
                        let wait = 5 * 2u64.pow(attempt);
                        tracing::warn!(attempt, wait, "Discogs broker transport error: {msg}");
                        Some(wait)
                    }
                    _ => None,
                };

                match backoff {
                    Some(secs) if attempt < MAX_ATTEMPTS - 1 => {
                        tokio::time::sleep(Duration::from_secs(secs)).await;
                    }
                    _ => return Err(e),
                }
            }
        }
    }

    unreachable!("loop always exits via return")
}

async fn cli_beatport_lookup_with_retry(
    client: &reqwest::Client,
    artist: &str,
    title: &str,
) -> Result<Option<beatport::BeatportResult>, String> {
    const MAX_ATTEMPTS: u32 = 4;

    for attempt in 0..MAX_ATTEMPTS {
        match beatport::lookup(client, artist, title).await {
            Ok(result) => return Ok(result),
            Err(e) => {
                let backoff = match &e {
                    beatport::BeatportError::Http {
                        status,
                        retry_after,
                        ..
                    } if status.as_u16() == 429 => {
                        let wait = retry_after
                            .as_deref()
                            .and_then(|s| s.parse::<u64>().ok())
                            .unwrap_or(5)
                            .min(120);
                        Some(wait)
                    }
                    beatport::BeatportError::Http { status, .. } if status.is_server_error() => {
                        Some(5 * 2u64.pow(attempt))
                    }
                    beatport::BeatportError::Request(_) => Some(5 * 2u64.pow(attempt)),
                    _ => None,
                };

                match backoff {
                    Some(secs) if attempt < MAX_ATTEMPTS - 1 => {
                        tokio::time::sleep(Duration::from_secs(secs)).await;
                    }
                    _ => return Err(e.to_string()),
                }
            }
        }
    }

    unreachable!("loop always exits via return")
}

async fn cli_analyze_for_hydrate(
    raw_file_path: &str,
    needs_stratum: bool,
    needs_essentia: bool,
    essentia_python: Option<&str>,
    cache_tx: &tokio::sync::mpsc::Sender<CacheWriteRequest<HydrateCacheMsg>>,
) -> HydrateAnalysisOutcome {
    let mut outcome = HydrateAnalysisOutcome::default();
    let file_path = match audio::resolve_audio_path(raw_file_path) {
        Ok(p) => p,
        Err(_) => {
            outcome.operation_failed = true;
            return outcome;
        }
    };
    let metadata = match std::fs::metadata(&file_path) {
        Ok(m) => m,
        Err(_) => {
            outcome.operation_failed = true;
            return outcome;
        }
    };
    let file_size = metadata.len() as i64;
    let file_mtime = file_mtime_unix(&metadata);
    if needs_stratum {
        let path_clone = file_path.clone();
        let decode_result =
            tokio::task::spawn_blocking(move || audio::decode_to_samples(&path_clone)).await;

        match decode_result {
            Ok(Ok((samples, sample_rate))) => {
                let path_for_grid = file_path.clone();
                let analysis = tokio::task::spawn_blocking(move || {
                    let input = audio::load_rekordbox_grid_input_for_path(&path_for_grid);
                    audio::analyze_with_stratum_input(&samples, sample_rate, input)
                })
                .await;

                match analysis {
                    Ok(Ok(analysis)) => {
                        let audio::StratumAnalysis {
                            result,
                            input_fingerprint,
                        } = analysis;
                        let features_json =
                            match serialize_cache_payload(&result, "stratum-dsp analysis") {
                                Ok(json) => json,
                                Err(e) => {
                                    tracing::error!("{e}");
                                    outcome.operation_failed = true;
                                    return outcome;
                                }
                            };
                        if let Err(e) = send_cache_message(
                            cache_tx,
                            HydrateCacheMsg::AudioAnalysis(CliCacheWriteMsg {
                                file_path: file_path.clone(),
                                analyzer: audio::ANALYZER_STRATUM.to_string(),
                                file_size,
                                file_mtime,
                                analyzer_version: audio::STRATUM_SCHEMA_VERSION.to_string(),
                                input_fingerprint,
                                features_json,
                            }),
                            "stratum-dsp analysis",
                        )
                        .await
                        {
                            tracing::error!("{e}");
                            outcome.cache_write_failed = true;
                        }
                    }
                    Ok(Err(e)) => {
                        tracing::error!("stratum-dsp analysis failed for {file_path}: {e}");
                        outcome.operation_failed = true;
                    }
                    Err(e) => {
                        tracing::error!("stratum-dsp analysis task failed for {file_path}: {e}");
                        outcome.operation_failed = true;
                    }
                }
            }
            Ok(Err(e)) => {
                tracing::error!("Audio decode failed for {file_path}: {e}");
                outcome.operation_failed = true;
            }
            Err(e) => {
                tracing::error!("Audio decode task failed for {file_path}: {e}");
                outcome.operation_failed = true;
            }
        }
    }

    if needs_essentia && let Some(python) = essentia_python {
        match audio::run_essentia(python, &file_path).await {
            Ok(features) => {
                let features_json = match serialize_cache_payload(&features, "essentia analysis") {
                    Ok(json) => json,
                    Err(e) => {
                        tracing::error!("{e}");
                        outcome.operation_failed = true;
                        return outcome;
                    }
                };
                if let Err(e) = send_cache_message(
                    cache_tx,
                    HydrateCacheMsg::AudioAnalysis(CliCacheWriteMsg {
                        file_path: file_path.clone(),
                        analyzer: audio::ANALYZER_ESSENTIA.to_string(),
                        file_size,
                        file_mtime,
                        analyzer_version: audio::ESSENTIA_SCHEMA_VERSION.to_string(),
                        input_fingerprint: String::new(),
                        features_json,
                    }),
                    "essentia analysis",
                )
                .await
                {
                    tracing::error!("{e}");
                    outcome.cache_write_failed = true;
                }
            }
            Err(e) => {
                tracing::error!("Essentia analysis failed for {file_path}: {e}");
                outcome.operation_failed = true;
            }
        }
    }

    outcome
}

#[cfg(test)]
mod batch_tests {
    use super::*;
    use crate::cli::async_test_support::{TEST_WATCHDOG, TaskGuard, bounded};

    fn enrichment_message(provider: &str, id: u32) -> HydrateCacheMsg {
        HydrateCacheMsg::Enrichment {
            provider: provider.to_string(),
            norm_artist: format!("artist-{id}"),
            norm_title: format!("title-{id}"),
            norm_album: None,
            match_quality: Some("exact".to_string()),
            response_json: Some("{}".to_string()),
        }
    }

    fn no_match_message(provider: &str, id: u32) -> HydrateCacheMsg {
        HydrateCacheMsg::Enrichment {
            provider: provider.to_string(),
            norm_artist: format!("artist-{id}"),
            norm_title: format!("title-{id}"),
            norm_album: None,
            match_quality: Some("none".to_string()),
            response_json: None,
        }
    }

    fn analysis_message(analyzer: &str, id: u32) -> HydrateCacheMsg {
        HydrateCacheMsg::AudioAnalysis(CliCacheWriteMsg {
            file_path: format!("/tmp/hydrate-{id}.flac"),
            analyzer: analyzer.to_string(),
            file_size: i64::from(id) + 1,
            file_mtime: i64::from(id) + 2,
            analyzer_version: "test-v1".to_string(),
            input_fingerprint: if analyzer == audio::ANALYZER_STRATUM {
                audio::STRATUM_HMM_INPUT_FINGERPRINT.to_string()
            } else {
                String::new()
            },
            features_json: "{}".to_string(),
        })
    }

    fn temp_store() -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("cache.sqlite3");
        let path = path.to_string_lossy().to_string();
        let conn = store::open(&path).expect("create store");
        drop(conn);
        (dir, path)
    }

    fn install_failure_triggers(path: &str) {
        let conn = store::open(path).expect("open store for triggers");
        conn.execute_batch(
            "CREATE TRIGGER reject_failed_enrichment
             BEFORE INSERT ON enrichment_cache
             WHEN NEW.provider = 'fail'
             BEGIN
               SELECT RAISE(FAIL, 'injected enrichment write failure');
             END;
             CREATE TRIGGER reject_failed_audio
             BEFORE INSERT ON audio_analysis_cache
             WHEN NEW.analyzer = 'fail'
             BEGIN
               SELECT RAISE(FAIL, 'injected audio write failure');
             END;",
        )
        .expect("install failure triggers");
    }

    async fn run_writer_requests(
        path: String,
        messages: Vec<HydrateCacheMsg>,
    ) -> (
        Vec<Result<(), String>>,
        CacheWriterReport,
        CancellationToken,
    ) {
        let cancel = CancellationToken::new();
        let writer_cancel = cancel.clone();
        let (tx, rx) = tokio::sync::mpsc::channel(messages.len().max(1));
        let writer = TaskGuard::new(tokio::task::spawn_blocking(move || {
            run_hydrate_cache_writer(path, rx, writer_cancel)
        }));
        let mut results = Vec::with_capacity(messages.len());
        for message in messages {
            match bounded(
                send_cache_message(&tx, message, "hydrate test"),
                "hydrate writer acknowledgement",
            )
            .await
            {
                Ok(result) => results.push(result),
                Err(error) => {
                    results.push(Err(error));
                    break;
                }
            }
        }
        drop(tx);
        let report = writer
            .join("hydrate writer join")
            .await
            .expect("guarded writer join");
        (results, report, cancel)
    }

    #[tokio::test]
    async fn hydrate_cache_writer_persists_both_payload_variants_before_ack() {
        tokio::time::timeout(TEST_WATCHDOG, async {
            let (_dir, path) = temp_store();
            let (results, report, cancel) = run_writer_requests(
                path.clone(),
                vec![no_match_message("discogs", 1), analysis_message("ok", 2)],
            )
            .await;
            assert_eq!(results, vec![Ok(()), Ok(())]);
            assert_eq!(report.attempted, 2);
            assert_eq!(report.succeeded, 2);
            assert_eq!(report.failed, 0);
            assert!(!cancel.is_cancelled());

            let conn = store::open(&path).expect("reopen store");
            let enrichment =
                store::get_enrichment(&conn, "discogs", "artist-1", "title-1", None, true)
                    .expect("read enrichment")
                    .expect("durable enrichment");
            assert_eq!(enrichment.match_quality.as_deref(), Some("none"));
            assert!(
                store::get_audio_analysis(&conn, "/tmp/hydrate-2.flac", "ok")
                    .expect("read analysis")
                    .is_some()
            );
            cancel.cancel();
        })
        .await
        .expect("hydrate writer success watchdog expired");
    }

    #[tokio::test]
    async fn hydrate_cache_writer_reports_selective_failure_and_recovers() {
        tokio::time::timeout(TEST_WATCHDOG, async {
            let (_dir, path) = temp_store();
            install_failure_triggers(&path);
            let (results, report, cancel) = run_writer_requests(
                path,
                vec![enrichment_message("fail", 1), analysis_message("ok", 2)],
            )
            .await;
            assert!(results[0].is_err());
            assert_eq!(results[1], Ok(()));
            assert_eq!(report.succeeded, 1);
            assert_eq!(report.failed, 1);
            assert!(!report.threshold_cancelled);
            assert!(!cancel.is_cancelled());
            cancel.cancel();
        })
        .await
        .expect("hydrate selective-failure watchdog expired");
    }

    #[tokio::test]
    async fn hydrate_cache_writer_threshold_cancels_and_drains_mixed_payloads() {
        tokio::time::timeout(TEST_WATCHDOG, async {
            let (_dir, path) = temp_store();
            install_failure_triggers(&path);
            let (results, report, cancel) = run_writer_requests(
                path,
                vec![
                    enrichment_message("fail", 1),
                    enrichment_message("fail", 2),
                    enrichment_message("fail", 3),
                    analysis_message("ok", 4),
                ],
            )
            .await;
            assert!(results.iter().all(Result::is_err));
            assert_eq!(report.attempted, 4);
            assert_eq!(report.failed, 4);
            assert!(report.threshold_cancelled);
            assert!(cancel.is_cancelled());
            assert!(
                results[3]
                    .as_ref()
                    .expect_err("drained payload rejection")
                    .contains("cache writer stopped")
            );
        })
        .await
        .expect("hydrate threshold watchdog expired");
    }

    #[tokio::test]
    async fn hydrate_cache_writer_open_failure_rejects_and_drains() {
        tokio::time::timeout(TEST_WATCHDOG, async {
            let dir = tempfile::tempdir().expect("temp dir");
            let invalid_path = dir.path().to_string_lossy().to_string();
            let (results, report, cancel) =
                run_writer_requests(invalid_path, vec![enrichment_message("ok", 1)]).await;
            assert!(
                results[0]
                    .as_ref()
                    .expect_err("open failure")
                    .contains("cache store open failed")
            );
            assert_eq!(report.attempted, 1);
            assert_eq!(report.failed, 1);
            assert!(cancel.is_cancelled());
        })
        .await
        .expect("hydrate open-failure watchdog expired");
    }

    #[tokio::test]
    async fn hydrate_cache_writer_tolerates_dropped_ack_receiver() {
        tokio::time::timeout(TEST_WATCHDOG, async {
            let (_dir, path) = temp_store();
            let cancel = CancellationToken::new();
            let writer_cancel = cancel.clone();
            let (tx, rx) = tokio::sync::mpsc::channel(1);
            let writer = TaskGuard::new(tokio::task::spawn_blocking(move || {
                run_hydrate_cache_writer(path, rx, writer_cancel)
            }));
            let (acknowledgement, result) = tokio::sync::oneshot::channel();
            drop(result);
            let send_result = bounded(
                tx.send(CacheWriteRequest {
                    payload: enrichment_message("ok", 1),
                    acknowledgement,
                }),
                "hydrate dropped-ack queue send",
            )
            .await;
            drop(tx);
            let report = writer.join("hydrate dropped-ack writer join").await;
            send_result
                .expect("bounded queue send")
                .expect("queue request");
            let report = report.expect("guarded writer join");
            assert_eq!(report.succeeded, 1);
            assert!(!cancel.is_cancelled());
            cancel.cancel();
        })
        .await
        .expect("hydrate dropped-ack watchdog expired");
    }

    #[tokio::test]
    async fn hydrate_provider_inner_and_outer_join_failures_are_incomplete() {
        tokio::time::timeout(TEST_WATCHDOG, async {
            let counters = ProviderCounters::new();
            let mut inner_report = ProviderTaskReport::new(1);
            let inner = TaskGuard::new(tokio::spawn(async {
                panic!("injected inner provider panic")
            }));
            let inner_result = inner
                .join_raw("injected inner provider join")
                .await
                .expect("bounded inner join");
            record_provider_worker_join("discogs", inner_result, &counters, &mut inner_report);
            assert_eq!(inner_report.join_failures, 1);
            assert_eq!(inner_report.incomplete(), 1);
            assert_eq!(
                inner_report.error_summaries,
                vec!["discogs worker task panicked"]
            );
            assert_eq!(counters.errors.load(Ordering::Relaxed), 1);
            assert_eq!(counters.operation_errors.load(Ordering::Relaxed), 0);

            counters.terminal.store(1, Ordering::Relaxed);
            let outer = TaskGuard::new(tokio::spawn(async {
                panic!("injected outer provider panic");
                #[allow(unreachable_code)]
                ProviderTaskReport::new(3)
            }));
            let outer_result = outer
                .join_raw("injected outer provider join")
                .await
                .expect("bounded outer join");
            let outer_report = resolve_provider_task_join("discogs", 3, &counters, outer_result);
            assert_eq!(outer_report.join_failures, 1);
            assert_eq!(outer_report.terminal_workers, 1);
            assert_eq!(outer_report.incomplete(), 2);
            assert_eq!(
                outer_report.error_summaries,
                vec!["discogs provider task panicked"]
            );
        })
        .await
        .expect("provider-join watchdog expired");
    }

    #[test]
    fn hydrate_batch_outcome_has_disjoint_exact_counts() {
        assert!(hydrate_batch_outcome(0, 0, 0, 0, false, vec![]).is_ok());

        let provider =
            hydrate_batch_outcome(2, 0, 0, 0, false, vec![]).expect_err("provider failure");
        assert_eq!(provider.track_or_provider_failures, 2);
        assert_eq!(provider.worker_join_failures, 0);
        assert_eq!(provider.writer_failures, 0);

        let join = hydrate_batch_outcome(0, 1, 0, 1, false, vec![]).expect_err("join failure");
        assert_eq!(join.track_or_provider_failures, 0);
        assert_eq!(join.worker_join_failures, 1);
        assert_eq!(join.writer_failures, 0);
        assert_eq!(join.incomplete, 1);

        let writer = hydrate_batch_outcome(0, 0, 3, 0, false, vec![]).expect_err("writer failure");
        assert_eq!(writer.track_or_provider_failures, 0);
        assert_eq!(writer.worker_join_failures, 0);
        assert_eq!(writer.writer_failures, 3);

        let cancelled =
            hydrate_batch_outcome(0, 0, 0, 2, true, vec![]).expect_err("cancelled batch");
        assert_eq!(cancelled.incomplete, 2);
        assert!(cancelled.user_cancelled);
    }
}
