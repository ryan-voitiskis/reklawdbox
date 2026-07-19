//! CLI hydration arguments and capability modules.

mod command;
mod discogs;
mod presentation;

#[cfg(test)]
mod tests;

use super::runtime::resources::CpuPreset;
use crate::application::enrichment::model::HydrationStages;

fn parse_providers(value: &str) -> Result<HydrationStages, String> {
    HydrationStages::parse_csv(value)
}

#[derive(clap::Args)]
pub(crate) struct HydrateArgs {
    /// Providers to run (comma-separated: discogs,analysis)
    #[arg(long, default_value = "discogs,analysis", value_parser = parse_providers)]
    pub(super) providers: HydrationStages,
    /// Filter by playlist ID
    #[arg(long)]
    pub(super) playlist: Option<String>,
    /// Filter by artist name (partial match)
    #[arg(long)]
    pub(super) artist: Option<String>,
    /// Filter by genre name (partial match)
    #[arg(long)]
    pub(super) genre: Option<String>,
    /// Minimum BPM
    #[arg(long)]
    pub(super) bpm_min: Option<f64>,
    /// Maximum BPM
    #[arg(long)]
    pub(super) bpm_max: Option<f64>,
    /// Filter by musical key
    #[arg(long)]
    pub(super) key: Option<String>,
    /// Filter by label name (partial match)
    #[arg(long)]
    pub(super) label: Option<String>,
    /// Filter by file path/folder (partial match)
    #[arg(long)]
    pub(super) path: Option<String>,
    /// Search query matching title or artist
    #[arg(long)]
    pub(super) query: Option<String>,
    /// Only tracks added on or after this date (ISO date)
    #[arg(long)]
    pub(super) added_after: Option<String>,
    /// Only tracks added on or before this date (ISO date)
    #[arg(long)]
    pub(super) added_before: Option<String>,
    /// Minimum star rating (1-5)
    #[arg(long)]
    pub(super) rating_min: Option<u8>,
    /// Max tracks to process (omit for unlimited)
    #[arg(long)]
    pub(super) max_tracks: Option<u32>,
    /// Don't retry previously-errored enrichments
    #[arg(long)]
    pub(super) no_retry_errors: bool,
    /// CPU scheduling preset for audio analysis
    #[arg(long, value_enum, default_value_t = CpuPreset::Background)]
    pub(super) cpu: CpuPreset,
    /// Enrichment concurrency (default: 4)
    #[arg(long, short = 'j')]
    pub(super) concurrency: Option<u32>,
    /// Skip confirmation prompt
    #[arg(long, short = 'y')]
    pub(super) yes: bool,
}

pub(crate) use command::run_hydrate;
