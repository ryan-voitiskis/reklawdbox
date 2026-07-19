//! Hydration estimates, prompt, progress, and final human summary.

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use console::style;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

use crate::application::cache_writer::CacheWriterReport;
use crate::application::enrichment::hydrate::{
    EnrichmentTrackOutcome, HydrationAnalysisOutcome, HydrationFailureKind, HydrationPlan,
    HydrationStageReport, HydrationTrackIdentity,
};
use crate::cli::runtime::resources::{
    CpuPreset, cpu_preset_summary, memory_preset_summary, track_memory_cost_mb,
};

pub(super) struct StartupPresentation<'a> {
    pub(super) plan: &'a HydrationPlan,
    pub(super) want_discogs: bool,
    pub(super) want_analysis: bool,
    pub(super) retry_errors: bool,
    pub(super) cpu_preset: CpuPreset,
    pub(super) analysis_concurrency: usize,
    pub(super) analysis_budget_mb: u32,
    pub(super) essentia_available: bool,
}

pub(super) fn print_startup(input: StartupPresentation<'_>) {
    let StartupPresentation {
        plan,
        want_discogs,
        want_analysis,
        retry_errors,
        cpu_preset,
        analysis_concurrency,
        analysis_budget_mb,
        essentia_available,
    } = input;
    println!(
        "Found {} tracks matching filters.",
        plan.total_matched_tracks
    );
    println!("  {}", cpu_preset_summary(cpu_preset, analysis_concurrency));
    if want_analysis {
        println!("  {}", memory_preset_summary(analysis_budget_mb));
    }
    if want_discogs {
        let retry_note = if plan.discogs_errors > 0 && retry_errors {
            format!(", {} errors to retry", plan.discogs_errors)
        } else if plan.discogs_errors > 0 {
            format!(", {} errors (skipped)", plan.discogs_errors)
        } else {
            String::new()
        };
        println!(
            "  Discogs:  {} cached{}, {} pending",
            plan.discogs_cached,
            retry_note,
            plan.discogs_tracks.len()
        );
    }
    if want_analysis {
        let essentia_note = if essentia_available {
            ""
        } else {
            " (stratum-dsp only)"
        };
        println!(
            "  Analysis: {} cached, {} pending{}",
            plan.analysis_cached,
            plan.analysis_jobs.len(),
            essentia_note,
        );
    }

    let discogs_secs = plan.discogs_tracks.len() as u64;
    let secs_per_analysis: u64 = if essentia_available { 48 } else { 18 };
    let effective_analysis_concurrency = if plan.analysis_jobs.is_empty() {
        analysis_concurrency
    } else {
        let avg_cost_mb = plan
            .analysis_jobs
            .iter()
            .map(|job| track_memory_cost_mb(job.track.length).min(analysis_budget_mb) as u64)
            .sum::<u64>()
            / plan.analysis_jobs.len() as u64;
        let memory_concurrency = (analysis_budget_mb as u64 / avg_cost_mb.max(1)) as usize;
        analysis_concurrency.min(memory_concurrency).max(1)
    };
    let analysis_secs = (plan.analysis_jobs.len() as u64 * secs_per_analysis)
        / effective_analysis_concurrency.max(1) as u64;
    let estimated_secs = discogs_secs.max(analysis_secs);
    if estimated_secs > 60 {
        let hours = estimated_secs / 3600;
        let minutes = (estimated_secs % 3600) / 60;
        if hours > 0 {
            println!("\nEstimated time: ~{hours}h{minutes:02}m");
        } else {
            println!("\nEstimated time: ~{minutes}m");
        }
    }
}

pub(super) fn confirm(skip: bool) -> Result<bool, std::io::Error> {
    if skip {
        return Ok(true);
    }
    print!("Continue? [Y/n] ");
    use std::io::Write;
    std::io::stdout().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let trimmed = input.trim().to_ascii_lowercase();
    Ok(trimmed.is_empty() || trimmed == "y" || trimmed == "yes")
}

pub(super) struct ProgressDisplay {
    pub(super) multi: MultiProgress,
    pub(super) primary: ProgressBar,
    pub(super) status: ProgressBar,
}

impl ProgressDisplay {
    pub(super) fn new(total_work: usize) -> Self {
        let multi = MultiProgress::new();
        let primary = multi.add(ProgressBar::new(total_work as u64));
        primary.set_style(
            ProgressStyle::with_template(
                "Hydrating [{bar:40.cyan/blue}] {pos}/{len}  {percent}%  ETA {eta}",
            )
            .unwrap()
            .progress_chars("##-"),
        );
        let status = multi.add(ProgressBar::new_spinner());
        status.set_style(ProgressStyle::with_template("  {msg}").unwrap());
        status.enable_steady_tick(Duration::from_secs(1));
        Self {
            multi,
            primary,
            status,
        }
    }

    pub(super) fn finish(&self) {
        self.status.finish_and_clear();
        self.primary.finish_and_clear();
        self.multi.clear().ok();
    }
}

pub(super) struct ProviderCounters {
    enriched: AtomicU32,
    no_match: AtomicU32,
    errors: AtomicU32,
    operation_errors: AtomicU32,
    terminal: AtomicU32,
}

impl ProviderCounters {
    pub(super) fn new() -> Self {
        Self {
            enriched: AtomicU32::new(0),
            no_match: AtomicU32::new(0),
            errors: AtomicU32::new(0),
            operation_errors: AtomicU32::new(0),
            terminal: AtomicU32::new(0),
        }
    }

    pub(super) fn observe_discogs(
        &self,
        identity: &HydrationTrackIdentity,
        outcome: &EnrichmentTrackOutcome,
    ) {
        if let Some(failure) = outcome.failures.first() {
            let error = hydration_failure_message(&failure.kind);
            if matches!(
                &failure.kind,
                HydrationFailureKind::Lookup(_) | HydrationFailureKind::DiscogsAuth(_)
            ) {
                tracing::error!(
                    "Discogs hydrate lookup failed for {} - {}: {error}",
                    identity.artist,
                    identity.title
                );
            } else {
                tracing::error!("{error}");
            }
        }

        self.enriched
            .fetch_add(outcome.enriched as u32, Ordering::Relaxed);
        self.no_match
            .fetch_add(outcome.no_match as u32, Ordering::Relaxed);
        self.errors
            .fetch_add(u32::from(!outcome.failures.is_empty()), Ordering::Relaxed);
        self.operation_errors
            .fetch_add(outcome.operation_failures as u32, Ordering::Relaxed);
    }

    pub(super) fn observe_analysis(&self, outcome: &HydrationAnalysisOutcome) {
        if outcome.succeeded() {
            self.enriched.fetch_add(1, Ordering::Relaxed);
        } else {
            self.errors.fetch_add(1, Ordering::Relaxed);
        }
        if outcome.operation_failed {
            self.operation_errors.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(super) fn record_join_failure(&self) {
        self.errors.fetch_add(1, Ordering::Relaxed);
    }

    pub(super) fn record_terminal(&self) {
        self.terminal.fetch_add(1, Ordering::Relaxed);
    }

    pub(super) fn enriched(&self) -> u32 {
        self.enriched.load(Ordering::Relaxed)
    }

    pub(super) fn no_match(&self) -> u32 {
        self.no_match.load(Ordering::Relaxed)
    }

    pub(super) fn errors(&self) -> u32 {
        self.errors.load(Ordering::Relaxed)
    }

    pub(super) fn operation_errors(&self) -> u32 {
        self.operation_errors.load(Ordering::Relaxed)
    }

    pub(super) fn terminal(&self) -> u32 {
        self.terminal.load(Ordering::Relaxed)
    }
}

pub(super) fn hydration_failure_message(kind: &HydrationFailureKind) -> String {
    match kind {
        HydrationFailureKind::AuthBatchFailed => "Discogs auth failed (batch-wide)".to_string(),
        HydrationFailureKind::DiscogsAuth(remediation) => {
            if let Some(auth_url) = remediation.auth_url.as_deref() {
                format!("{} Auth URL: {auth_url}", remediation.message)
            } else {
                remediation.message.clone()
            }
        }
        HydrationFailureKind::Lookup(error) => error.clone(),
        HydrationFailureKind::Serialize(error) => {
            format!("discogs enrichment cache serialization failed: {error}")
        }
        HydrationFailureKind::SemaphoreClosed => "discogs semaphore closed".to_string(),
        HydrationFailureKind::CacheWrite(error) => error.to_string(),
    }
}

#[derive(Clone, Copy)]
pub(super) struct FinalPresentation<'a> {
    pub(super) elapsed: Duration,
    pub(super) want_discogs: bool,
    pub(super) want_analysis: bool,
    pub(super) discogs: &'a HydrationStageReport,
    pub(super) analysis: &'a HydrationStageReport,
    pub(super) writer: &'a CacheWriterReport,
    pub(super) writer_join_failures: u32,
    pub(super) incomplete: usize,
    pub(super) user_cancelled: bool,
}

pub(super) fn final_lines(input: FinalPresentation<'_>) -> Vec<String> {
    let minutes = input.elapsed.as_secs() / 60;
    let seconds = input.elapsed.as_secs() % 60;
    let mut lines = vec![format!("\nDone ({minutes}m {seconds}s)")];
    if input.want_discogs {
        let errors = input.discogs.failed + input.discogs.worker_join_failures;
        lines.push(format!(
            "  Discogs:  {} enriched, {} no match, {} errors",
            style(input.discogs.enriched).green(),
            style(input.discogs.no_match).dim(),
            if errors > 0 {
                style(errors).red()
            } else {
                style(errors).dim()
            },
        ));
    }
    if input.want_analysis {
        let errors = input.analysis.failed + input.analysis.worker_join_failures;
        lines.push(format!(
            "  Analysis: {} done, {} errors",
            style(input.analysis.enriched).green(),
            if errors > 0 {
                style(errors).red()
            } else {
                style(errors).dim()
            },
        ));
    }
    if input.writer.attempted > 0 || input.writer.failed > 0 {
        lines.push(format!(
            "  Cache writes: {} succeeded, {} failed",
            style(input.writer.succeeded).green(),
            if input.writer.failed > 0 {
                style(input.writer.failed).red()
            } else {
                style(input.writer.failed).dim()
            }
        ));
    }
    if input.writer_join_failures > 0 {
        lines.push(format!("  Cache writer task: {}", style("failed").red()));
    }
    if input.incomplete > 0 {
        lines.push(format!(
            "  Incomplete: {} selected tasks",
            style(input.incomplete).red()
        ));
    }
    if input.user_cancelled {
        lines.push("Cancelled".to_string());
    }
    lines
}

pub(super) fn print_final(input: FinalPresentation<'_>) {
    for line in final_lines(input) {
        println!("{line}");
    }
}
