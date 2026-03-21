use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use tokio_util::sync::CancellationToken;

use console::style;

use crate::{audio, beatport, db, discogs, normalize, store, tools};

use super::{
    CliCacheWriteMsg, cache_probe_for_path, cache_status_for_track, file_mtime_unix,
    send_cache_message, serialize_cache_payload,
};

// ---------------------------------------------------------------------------
// CLI args
// ---------------------------------------------------------------------------

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
    /// Filter by playlist name
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

// ---------------------------------------------------------------------------
// Counters
// ---------------------------------------------------------------------------

struct ProviderCounters {
    enriched: AtomicU32,
    errors: AtomicU32,
}

impl ProviderCounters {
    fn new() -> Self {
        Self {
            enriched: AtomicU32::new(0),
            errors: AtomicU32::new(0),
        }
    }
}

// ---------------------------------------------------------------------------
// Cache write messages
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Main orchestrator
// ---------------------------------------------------------------------------

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
    let client = reqwest::Client::new();

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
            match store::get_enrichment(&store_conn, "beatport", &norm_artist, &norm_title, None)? {
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
    analysis_pending.sort_by(|a, b| b.0.length.cmp(&a.0.length));

    drop(store_conn);

    let total_tracks = tracks.len();
    let total_work = discogs_pending.len() + beatport_pending.len() + analysis_pending.len();

    if total_work == 0 {
        println!("Found {total_tracks} tracks matching filters.");
        println!("All cached. Nothing to do.");
        return Ok(());
    }

    // 4. Startup summary
    println!("Found {} tracks matching filters.", total_tracks);
    println!(
        "  {}",
        super::cpu_preset_summary(cpu_preset, analysis_concurrency)
    );
    if want_analysis {
        println!("  {}", super::memory_preset_summary(analysis_budget_mb));
    }
    if want_discogs {
        let retry_note = if discogs_errors > 0 && retry_errors {
            format!(", {} errors to retry", discogs_errors)
        } else if discogs_errors > 0 {
            format!(", {} errors (skipped)", discogs_errors)
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
            format!(", {} errors to retry", beatport_errors)
        } else if beatport_errors > 0 {
            format!(", {} errors (skipped)", beatport_errors)
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

    // Estimate time
    let enrich_concurrency_est = args.concurrency.unwrap_or(4).max(1) as u64;
    let beatport_secs = beatport_pending.len() as u64; // ~1 req/s rate limit
    let discogs_secs = discogs_pending.len() as u64 / enrich_concurrency_est;
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
            println!("\nEstimated time: ~{}h{:02}m", hours, mins);
        } else {
            println!("\nEstimated time: ~{}m", mins);
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

    // Ctrl+C handler
    let cancel_clone = cancel.clone();
    let mp_clone = mp.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            mp_clone
                .println("Shutting down gracefully... (waiting for in-flight tasks)")
                .ok();
            cancel_clone.cancel();
        }
    });

    // Counters
    let discogs_counters = Arc::new(ProviderCounters::new());
    let beatport_counters = Arc::new(ProviderCounters::new());
    let analysis_counters = Arc::new(ProviderCounters::new());

    // 8. Cache writer task
    let enrich_concurrency = args.concurrency.unwrap_or(4).clamp(1, 16) as usize;
    let (cache_tx, mut cache_rx) =
        tokio::sync::mpsc::channel::<HydrateCacheMsg>(enrich_concurrency * 8 + 32);

    let writer_store_path = store_path_str.clone();
    let writer_cancel = cancel.clone();
    let writer_handle = tokio::task::spawn_blocking(move || {
        let conn = match store::open(&writer_store_path) {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("Cache writer: failed to open store: {e} — aborting pipeline");
                writer_cancel.cancel();
                return;
            }
        };
        let mut consecutive_failures: u32 = 0;
        while let Some(msg) = cache_rx.blocking_recv() {
            let write_result = match &msg {
                HydrateCacheMsg::Enrichment {
                    provider,
                    norm_artist,
                    norm_title,
                    norm_album,
                    match_quality,
                    response_json,
                } => store::set_enrichment(
                    &conn,
                    provider,
                    norm_artist,
                    norm_title,
                    norm_album.as_deref(),
                    match_quality.as_deref(),
                    response_json.as_deref(),
                ),
                HydrateCacheMsg::AudioAnalysis(a) => store::set_audio_analysis(
                    &conn,
                    &a.file_path,
                    &a.analyzer,
                    a.file_size,
                    a.file_mtime,
                    &a.analyzer_version,
                    &a.features_json,
                ),
            };
            if let Err(e) = write_result {
                consecutive_failures += 1;
                let label = match &msg {
                    HydrateCacheMsg::Enrichment {
                        provider,
                        norm_artist,
                        norm_title,
                        ..
                    } => format!("{provider} {norm_artist}/{norm_title}"),
                    HydrateCacheMsg::AudioAnalysis(a) => {
                        format!("{} {}", a.analyzer, a.file_path)
                    }
                };
                tracing::error!(
                    "Cache writer: write failed for {label}: {e} ({consecutive_failures}/{})",
                    super::MAX_CONSECUTIVE_CACHE_WRITE_FAILURES,
                );
                if consecutive_failures >= super::MAX_CONSECUTIVE_CACHE_WRITE_FAILURES {
                    tracing::error!(
                        "Cache writer: {consecutive_failures} consecutive failures — aborting pipeline"
                    );
                    writer_cancel.cancel();
                    return;
                }
            } else {
                consecutive_failures = 0;
            }
        }
    });

    // 9. Spawn provider loops concurrently
    // Each provider runs in its own task so serial Beatport doesn't block analysis spawning.

    // Status updater
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

    // Discogs producer task
    let discogs_task = {
        let cancel = cancel.clone();
        let client = client.clone();
        let cache_tx = cache_tx.clone();
        let counters = discogs_counters.clone();
        let pb = pb.clone();
        tokio::spawn(async move {
            if discogs_pending.is_empty() {
                return;
            }
            let (broker_cfg, session_token) = match discogs_mode {
                Some(m) => m,
                None => return,
            };
            let sem = Arc::new(tokio::sync::Semaphore::new(enrich_concurrency));
            let mut handles = Vec::with_capacity(discogs_pending.len());

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

                handles.push(tokio::spawn(async move {
                    if cancel.is_cancelled() {
                        drop(permit);
                        return;
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
                                counters.enriched.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        Err(err) => {
                            tracing::error!(
                                "Discogs hydrate lookup failed for {} - {}: {err}",
                                track.artist,
                                track.title
                            );
                            counters.errors.fetch_add(1, Ordering::Relaxed);
                        }
                    }

                    pb.inc(1);
                    drop(permit);
                }));
            }

            for handle in handles {
                let _ = handle.await;
            }
        })
    };

    // Beatport producer task (serial — semaphore=1)
    let beatport_task = {
        let cancel = cancel.clone();
        let client = client.clone();
        let cache_tx = cache_tx.clone();
        let counters = beatport_counters.clone();
        let pb = pb.clone();
        tokio::spawn(async move {
            if beatport_pending.is_empty() {
                return;
            }
            let sem = Arc::new(tokio::sync::Semaphore::new(1));
            let mut handles = Vec::with_capacity(beatport_pending.len());

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

                handles.push(tokio::spawn(async move {
                    if cancel.is_cancelled() {
                        drop(permit);
                        return;
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
                                            match_quality: Some("exact".to_string()),
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
                                counters.enriched.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        Err(err) => {
                            tracing::error!(
                                "Beatport hydrate lookup failed for {} - {}: {err}",
                                track.artist,
                                track.title
                            );
                            counters.errors.fetch_add(1, Ordering::Relaxed);
                        }
                    }

                    pb.inc(1);
                    drop(permit);
                }));
            }

            for handle in handles {
                let _ = handle.await;
            }
        })
    };

    // Analysis producer task (dual semaphore: CPU + memory)
    let analysis_task = {
        let cancel = cancel.clone();
        let cache_tx = cache_tx.clone();
        let counters = analysis_counters.clone();
        let pb = pb.clone();
        tokio::spawn(async move {
            if analysis_pending.is_empty() {
                return;
            }
            let cpu_sem = Arc::new(tokio::sync::Semaphore::new(analysis_concurrency));
            let mem_sem = Arc::new(tokio::sync::Semaphore::new(analysis_budget_mb as usize));
            let mut handles = Vec::with_capacity(analysis_pending.len());

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

                handles.push(tokio::spawn(async move {
                    if cancel.is_cancelled() {
                        drop(cpu_permit);
                        drop(mem_permit);
                        return;
                    }

                    let ok = cli_analyze_for_hydrate(
                        &track.file_path,
                        needs_stratum,
                        needs_essentia,
                        essentia_python.as_deref(),
                        &cache_tx,
                    )
                    .await;

                    if ok {
                        counters.enriched.fetch_add(1, Ordering::Relaxed);
                    } else {
                        counters.errors.fetch_add(1, Ordering::Relaxed);
                    }

                    pb.inc(1);
                    drop(cpu_permit);
                    drop(mem_permit);
                }));
            }

            for handle in handles {
                let _ = handle.await;
            }
        })
    };

    // Drop our sender so the writer sees EOF when all producer tasks finish
    drop(cache_tx);

    // Await all producer tasks
    let _ = tokio::join!(discogs_task, beatport_task, analysis_task);

    // Stop status updates
    cancel.cancel();
    let _ = status_task.await;

    // Wait for writer to flush
    let _ = writer_handle.await;

    // Clear progress bars
    mp.clear().ok();

    // 10. Summary
    let elapsed = batch_start.elapsed();
    let mins = elapsed.as_secs() / 60;
    let secs = elapsed.as_secs() % 60;

    println!("\nDone ({mins}m {secs}s)");
    if want_discogs {
        let enriched = discogs_counters.enriched.load(Ordering::Relaxed);
        let errors = discogs_counters.errors.load(Ordering::Relaxed);
        println!(
            "  Discogs:  {} enriched, {} errors",
            style(enriched).green(),
            if errors > 0 {
                style(errors).red()
            } else {
                style(errors).dim()
            },
        );
    }
    if want_beatport {
        let enriched = beatport_counters.enriched.load(Ordering::Relaxed);
        let errors = beatport_counters.errors.load(Ordering::Relaxed);
        println!(
            "  Beatport: {} enriched, {} errors",
            style(enriched).green(),
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

    Ok(())
}

// ---------------------------------------------------------------------------
// Discogs auth helper
// ---------------------------------------------------------------------------

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

            // Start device auth flow
            println!("Discogs: starting broker authentication...");
            let pending = discogs::device_session_start(client, &cfg)
                .await
                .map_err(|e| format!("Failed to start Discogs auth: {e}"))?;

            println!("Please authorize at: {}", pending.auth_url);
            // Try to open in browser (macOS convenience)
            let _ = std::process::Command::new("open")
                .arg(&pending.auth_url)
                .spawn();

            // Poll with spinner
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

// ---------------------------------------------------------------------------
// Discogs retry wrapper
// ---------------------------------------------------------------------------

async fn cli_discogs_lookup_with_retry(
    client: &reqwest::Client,
    cfg: &discogs::BrokerConfig,
    token: &str,
    artist: &str,
    title: &str,
    album: Option<&str>,
) -> Result<Option<discogs::DiscogsResult>, discogs::LookupError> {
    match discogs::lookup_via_broker(client, cfg, token, artist, title, album).await {
        Ok(result) => Ok(result),
        // The broker handles Discogs 429s internally and returns 502 on
        // upstream failure, so this arm is defence-in-depth for platform-
        // level rate limits (e.g. Cloudflare) or custom broker setups.
        Err(discogs::LookupError::Http {
            status: 429,
            ref retry_after,
            ref body,
            ..
        }) => {
            let wait = retry_after
                .as_deref()
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(5)
                .min(120);
            tracing::warn!(status = 429, wait, "Discogs broker 429, retrying: {body}");
            tokio::time::sleep(Duration::from_secs(wait)).await;
            discogs::lookup_via_broker(client, cfg, token, artist, title, album).await
        }
        Err(discogs::LookupError::Http {
            status, ref body, ..
        }) if (500..=599).contains(&status) => {
            tracing::warn!(status, "Discogs broker {status}, retrying: {body}");
            tokio::time::sleep(Duration::from_secs(5)).await;
            discogs::lookup_via_broker(client, cfg, token, artist, title, album).await
        }
        Err(e) => Err(e),
    }
}

// ---------------------------------------------------------------------------
// Beatport retry wrapper
// ---------------------------------------------------------------------------

async fn cli_beatport_lookup_with_retry(
    client: &reqwest::Client,
    artist: &str,
    title: &str,
) -> Result<Option<beatport::BeatportResult>, String> {
    match beatport::lookup(client, artist, title).await {
        Ok(result) => Ok(result),
        Err(beatport::BeatportError::Http {
            status,
            retry_after,
            ..
        }) if status.as_u16() == 429 => {
            // Parse Retry-After or default to 5s
            let wait = retry_after
                .as_deref()
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(5);
            tokio::time::sleep(Duration::from_secs(wait)).await;
            beatport::lookup(client, artist, title)
                .await
                .map_err(|e| e.to_string())
        }
        Err(beatport::BeatportError::Http { status, .. }) if status.is_server_error() => {
            tokio::time::sleep(Duration::from_secs(5)).await;
            beatport::lookup(client, artist, title)
                .await
                .map_err(|e| e.to_string())
        }
        Err(beatport::BeatportError::Request(_)) => {
            tokio::time::sleep(Duration::from_secs(5)).await;
            beatport::lookup(client, artist, title)
                .await
                .map_err(|e| e.to_string())
        }
        Err(e) => Err(e.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Analysis helper (reuses analyze module's pattern)
// ---------------------------------------------------------------------------

async fn cli_analyze_for_hydrate(
    raw_file_path: &str,
    needs_stratum: bool,
    needs_essentia: bool,
    essentia_python: Option<&str>,
    cache_tx: &tokio::sync::mpsc::Sender<HydrateCacheMsg>,
) -> bool {
    let file_path = match audio::resolve_audio_path(raw_file_path) {
        Ok(p) => p,
        Err(_) => return false,
    };
    let metadata = match std::fs::metadata(&file_path) {
        Ok(m) => m,
        Err(_) => return false,
    };
    let file_size = metadata.len() as i64;
    let file_mtime = file_mtime_unix(&metadata);
    let mut ok = true;

    if needs_stratum {
        let path_clone = file_path.clone();
        let decode_result =
            tokio::task::spawn_blocking(move || audio::decode_to_samples(&path_clone)).await;

        match decode_result {
            Ok(Ok((samples, sample_rate))) => {
                let analysis = tokio::task::spawn_blocking(move || {
                    audio::analyze_with_stratum(&samples, sample_rate)
                })
                .await;

                match analysis {
                    Ok(Ok(result)) => {
                        let features_json =
                            match serialize_cache_payload(&result, "stratum-dsp analysis") {
                                Ok(json) => json,
                                Err(e) => {
                                    tracing::error!("{e}");
                                    ok = false;
                                    return ok;
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
                                features_json,
                            }),
                            "stratum-dsp analysis",
                        )
                        .await
                        {
                            tracing::error!("{e}");
                            ok = false;
                        }
                    }
                    _ => {
                        ok = false;
                    }
                }
            }
            _ => {
                ok = false;
            }
        }
    }

    if needs_essentia {
        if let Some(python) = essentia_python {
            match audio::run_essentia(python, &file_path).await {
                Ok(features) => {
                    let features_json =
                        match serialize_cache_payload(&features, "essentia analysis") {
                            Ok(json) => json,
                            Err(e) => {
                                tracing::error!("{e}");
                                ok = false;
                                return ok;
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
                            features_json,
                        }),
                        "essentia analysis",
                    )
                    .await
                    {
                        tracing::error!("{e}");
                        ok = false;
                    }
                }
                Err(_) => {
                    ok = false;
                }
            }
        }
    }

    ok
}
