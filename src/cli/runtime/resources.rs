//! CLI CPU and memory resource policy.

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

#[cfg(test)]
mod tests {
    use super::*;

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
