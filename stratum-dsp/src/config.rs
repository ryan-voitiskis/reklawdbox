//! Configuration parameters for audio analysis

use crate::analysis::result::BeatGrid;
use crate::error::AnalysisError;
use crate::features::key::templates::TemplateSet;
use crate::preprocessing::normalization::NormalizationMethod;

/// Maximum retained spectrogram cells per input sample.
///
/// Sixteen `f32` cells have a 64-byte raw floor per input sample before
/// per-vector and FFT overhead. This is twice the largest shipped STFT path
/// (the default key override is approximately eight cells per sample). The
/// bound applies independently to each computed spectrogram, not to the sum
/// of optional pipeline passes.
pub const MAX_SPECTROGRAM_CELLS_PER_INPUT_SAMPLE: usize = 16;

pub(crate) fn validate_spectrogram_request(
    sample_count: usize,
    frame_size: usize,
    hop_size: usize,
    frame_field: &str,
    hop_field: &str,
) -> Result<(usize, usize), AnalysisError> {
    if frame_size < 2 {
        return Err(AnalysisError::InvalidInput(format!(
            "{frame_field} must be at least 2, got {frame_size}"
        )));
    }
    if hop_size == 0 {
        return Err(AnalysisError::InvalidInput(format!(
            "{hop_field} must be greater than 0, got 0"
        )));
    }

    let n_bins = frame_size
        .checked_div(2)
        .and_then(|half| half.checked_add(1))
        .ok_or_else(|| {
            AnalysisError::InvalidInput(format!(
                "{frame_field} produces an unrepresentable FFT bin count"
            ))
        })?;
    let n_frames = if sample_count < frame_size {
        0
    } else {
        sample_count
            .checked_sub(frame_size)
            .and_then(|remaining| remaining.checked_div(hop_size))
            .and_then(|frames| frames.checked_add(1))
            .ok_or_else(|| {
                AnalysisError::InvalidInput(format!(
                    "{frame_field}/{hop_field} produce an unrepresentable frame count"
                ))
            })?
    };
    let cells = n_frames.checked_mul(n_bins).ok_or_else(|| {
        AnalysisError::InvalidInput(format!(
            "{frame_field}/{hop_field} produce an unrepresentable spectrogram cell count"
        ))
    })?;
    let budget = sample_count
        .checked_mul(MAX_SPECTROGRAM_CELLS_PER_INPUT_SAMPLE)
        .ok_or_else(|| {
            AnalysisError::InvalidInput(format!(
                "sample_count exceeds the spectrogram resource-policy arithmetic range: {sample_count}"
            ))
        })?;
    if cells > budget {
        return Err(AnalysisError::InvalidInput(format!(
            "{frame_field}/{hop_field} request {cells} spectrogram cells, exceeding the {budget}-cell budget for {sample_count} input samples"
        )));
    }

    Ok((n_frames, n_bins))
}

fn require_finite(field: &str, value: f32) -> Result<(), AnalysisError> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(AnalysisError::InvalidInput(format!(
            "{field} must be finite, got {value}"
        )))
    }
}

fn require_positive(field: &str, value: f32) -> Result<(), AnalysisError> {
    if value > 0.0 {
        Ok(())
    } else {
        Err(AnalysisError::InvalidInput(format!(
            "{field} must be greater than 0, got {value}"
        )))
    }
}

fn require_non_negative(field: &str, value: f32) -> Result<(), AnalysisError> {
    if value >= 0.0 {
        Ok(())
    } else {
        Err(AnalysisError::InvalidInput(format!(
            "{field} must be non-negative, got {value}"
        )))
    }
}

fn require_closed_unit(field: &str, value: f32) -> Result<(), AnalysisError> {
    if (0.0..=1.0).contains(&value) {
        Ok(())
    } else {
        Err(AnalysisError::InvalidInput(format!(
            "{field} must be in [0, 1], got {value}"
        )))
    }
}

fn require_window(field: &str, margin: usize) -> Result<(), AnalysisError> {
    margin
        .checked_mul(2)
        .and_then(|width| width.checked_add(1))
        .map(|_| ())
        .ok_or_else(|| {
            AnalysisError::InvalidInput(format!(
                "{field}={margin} overflows the 2 * margin + 1 window size"
            ))
        })
}

fn require_weight_group(field: &str, weights: &[f32]) -> Result<(), AnalysisError> {
    for (index, &weight) in weights.iter().enumerate() {
        require_finite(&format!("{field}[{index}]"), weight)?;
        require_non_negative(&format!("{field}[{index}]"), weight)?;
    }
    if weights.iter().all(|weight| *weight == 0.0) {
        return Err(AnalysisError::InvalidInput(format!(
            "{field} must contain at least one positive weight"
        )));
    }
    Ok(())
}

fn bpm_candidate_count(
    min_bpm: f32,
    max_bpm: f32,
    resolution: f32,
) -> Result<usize, AnalysisError> {
    let count = ((f64::from(max_bpm) - f64::from(min_bpm)) / f64::from(resolution)).ceil() + 1.0;
    if !count.is_finite() || count <= 0.0 || count >= usize::MAX as f64 {
        return Err(AnalysisError::InvalidInput(format!(
            "min_bpm/max_bpm/bpm_resolution produce an unrepresentable candidate count: {count}"
        )));
    }
    Ok(count as usize)
}

/// Analysis configuration parameters
#[derive(Debug, Clone)]
pub struct AnalysisConfig {
    // Preprocessing
    /// Silence detection threshold in dB (default: -40.0)
    /// Frames with RMS below this threshold are considered silent
    pub min_amplitude_db: f32,

    /// Normalization method to use (default: Peak)
    pub normalization: NormalizationMethod,

    /// Enable normalization step (default: true)
    pub enable_normalization: bool,

    /// Enable silence detection + trimming step (default: true)
    pub enable_silence_trimming: bool,

    // Onset detection (used by beat tracking and legacy BPM fallback)
    /// Enable multi-detector onset consensus (spectral flux + HFC + optional HPSS) (default: true)
    ///
    /// Note: Tempogram BPM does not use this onset list, but legacy BPM + beat tracking do.
    pub enable_onset_consensus: bool,

    /// Threshold percentile for STFT-based onset detectors (spectral flux / HFC / HPSS) (default: 0.80)
    /// Range: [0.0, 1.0]
    pub onset_threshold_percentile: f32,

    /// Onset clustering tolerance window in milliseconds for consensus voting (default: 50 ms)
    pub onset_consensus_tolerance_ms: u32,

    /// Consensus method weights [energy_flux, spectral_flux, hfc, hpss] (default: equal weights)
    pub onset_consensus_weights: [f32; 4],

    /// Enable HPSS-based onset detector inside consensus (default: false; more expensive)
    pub enable_hpss_onsets: bool,

    /// HPSS median-filter margin (default: 10). Typical values: 5–20.
    pub hpss_margin: usize,

    // BPM detection
    /// Force legacy BPM estimation (Phase 1B autocorrelation + comb filter) and skip tempogram.
    /// Default: false.
    ///
    /// Intended for A/B validation and hybrid/consensus experimentation.
    pub force_legacy_bpm: bool,

    /// Enable BPM fusion (compute tempogram + legacy in parallel, then choose using consensus logic).
    /// Default: false (tempogram-only unless it fails, then legacy fallback).
    pub enable_bpm_fusion: bool,

    /// Enable legacy BPM guardrails (soft confidence caps by tempo range).
    /// Default: true.
    pub enable_legacy_bpm_guardrails: bool,

    /// Enable **true** multi-resolution tempogram BPM estimation.
    ///
    /// When enabled, BPM estimation recomputes STFT at hop sizes {256, 512, 1024} and fuses
    /// candidates using a cross-resolution scoring rule. This is intended to reduce
    /// metrical-level (T vs 2T vs T/2) errors.
    ///
    /// Default: true (Phase 1F tuning path).
    pub enable_tempogram_multi_resolution: bool,

    /// Multi-resolution fusion: number of hop=512 candidates to consider as anchors.
    /// Default: 10.
    pub tempogram_multi_res_top_k: usize,

    /// Multi-resolution fusion weight for hop=512 (global beat).
    pub tempogram_multi_res_w512: f32,
    /// Multi-resolution fusion weight for hop=256 (fine transients).
    pub tempogram_multi_res_w256: f32,
    /// Multi-resolution fusion weight for hop=1024 (structural/metre level).
    pub tempogram_multi_res_w1024: f32,

    /// Structural discount factor applied when hop=1024 supports 2T instead of T.
    pub tempogram_multi_res_structural_discount: f32,

    /// Factor applied to hop=512 support when evaluating the 2T / T/2 hypotheses.
    pub tempogram_multi_res_double_time_512_factor: f32,

    /// Minimum score margin (absolute) required to switch between T / 2T / T/2 hypotheses.
    pub tempogram_multi_res_margin_threshold: f32,

    /// Enable a gentle human-tempo prior as a tie-breaker (only when scores are very close).
    /// Default: false.
    pub tempogram_multi_res_use_human_prior: bool,

    /// Enable HPSS percussive-only tempogram fallback (ambiguous-only).
    ///
    /// This computes an HPSS decomposition on the (already computed) STFT magnitudes and re-runs
    /// tempogram on the percussive component. Intended to reduce low-tempo half/double-time traps
    /// caused by sustained harmonic energy.
    ///
    /// Default: true (Phase 1F tuning path).
    pub enable_tempogram_percussive_fallback: bool,

    /// Enable multi-band novelty fusion inside the tempogram estimator.
    ///
    /// This computes novelty curves over low/mid/high frequency bands, runs the tempogram
    /// on each, then fuses their support when scoring BPM candidates. This is primarily
    /// intended to improve **candidate generation** (getting GT into top-N candidates),
    /// which is currently the limiting factor after metrical selection improvements.
    ///
    /// Default: true (Phase 1F tuning path).
    pub enable_tempogram_band_fusion: bool,

    /// Band split cutoffs (Hz). Bands are: low=[~0..low_max], mid=[low_max..mid_max], high=[mid_max..high_max].
    /// If `tempogram_band_high_max_hz <= 0`, high extends to Nyquist.
    pub tempogram_band_low_max_hz: f32,
    /// Upper cutoff for the mid band (Hz).
    pub tempogram_band_mid_max_hz: f32,
    /// Upper cutoff for the high band (Hz). If <= 0, uses Nyquist.
    pub tempogram_band_high_max_hz: f32,

    /// Weight for the full-band tempogram contribution when band-score fusion is enabled.
    pub tempogram_band_w_full: f32,
    /// Weight for the low band contribution.
    pub tempogram_band_w_low: f32,
    /// Weight for the mid band contribution.
    pub tempogram_band_w_mid: f32,
    /// Weight for the high band contribution.
    pub tempogram_band_w_high: f32,

    /// If true, multi-band tempograms contribute **only to candidate seeding** (peak proposals),
    /// while final candidate scoring remains full-band-only.
    ///
    /// This is the safer default: high-frequency bands often emphasize subdivisions (hi-hats),
    /// which can otherwise increase 2× / 3:2 metrical errors if they directly affect scoring.
    pub tempogram_band_seed_only: bool,

    /// Minimum per-band normalized support required to count as "supporting" a BPM candidate
    /// for band-consensus scoring.
    ///
    /// Range: [0, 1]. Default: 0.25.
    pub tempogram_band_support_threshold: f32,

    /// Bonus multiplier applied when **multiple bands** support the same BPM candidate.
    ///
    /// This is a lightweight "consensus" heuristic intended to reduce metrical/subdivision errors
    /// (e.g., a 2× tempo supported only by the high band should not win over a tempo supported by
    /// low+mid bands).
    ///
    /// Score adjustment: `score *= (1 + bonus * max(0, support_bands - 1))`.
    pub tempogram_band_consensus_bonus: f32,

    /// Tempogram novelty weights for combining {spectral, energy, HFC}.
    pub tempogram_novelty_w_spectral: f32,
    /// Tempogram novelty weight for energy flux.
    pub tempogram_novelty_w_energy: f32,
    /// Tempogram novelty weight for HFC.
    pub tempogram_novelty_w_hfc: f32,
    /// Tempogram novelty conditioning windows.
    pub tempogram_novelty_local_mean_window: usize,
    /// Tempogram novelty moving-average smoothing window (frames). Use 0/1 to disable.
    pub tempogram_novelty_smooth_window: usize,

    /// Debug: if set, the `analyze_file` example will pass this track ID through to the
    /// multi-resolution fusion so it can print detailed scoring diagnostics.
    pub debug_track_id: Option<u32>,
    /// Debug: optional ground-truth BPM passed alongside `debug_track_id`.
    pub debug_gt_bpm: Option<f32>,
    /// Debug: number of top candidates per hop to print when `debug_track_id` is set.
    pub debug_top_n: usize,

    /// Enable log-mel novelty tempogram as an additional candidate generator/support signal.
    ///
    /// This computes a log-mel SuperFlux-style novelty curve, then runs the tempogram on it.
    /// The resulting candidates are used for seeding and for the consensus bonus logic.
    pub enable_tempogram_mel_novelty: bool,
    /// Mel band count used by log-mel novelty.
    pub tempogram_mel_n_mels: usize,
    /// Minimum mel frequency (Hz).
    pub tempogram_mel_fmin_hz: f32,
    /// Maximum mel frequency (Hz). If <= 0, uses Nyquist.
    pub tempogram_mel_fmax_hz: f32,
    /// Max-filter neighborhood radius in mel bins (SuperFlux-style reference).
    pub tempogram_mel_max_filter_bins: usize,
    /// Weight for mel variant when band scoring fusion is enabled (`seed_only=false`).
    pub tempogram_mel_weight: f32,

    /// SuperFlux max-filter neighborhood radius (bins) used by the tempogram novelty extractor.
    pub tempogram_superflux_max_filter_bins: usize,

    /// Emit tempogram BPM candidate list (top-N) into `AnalysisMetadata` for validation/tuning.
    ///
    /// Default: false (avoid bloating outputs in normal use).
    pub emit_tempogram_candidates: bool,

    /// Number of tempogram candidates to emit when `emit_tempogram_candidates` is enabled.
    /// Default: 10.
    pub tempogram_candidates_top_n: usize,

    /// Legacy guardrails: preferred BPM range (default: 75–150).
    pub legacy_bpm_preferred_min: f32,
    /// Legacy guardrails: preferred BPM range upper bound (default: 150).
    pub legacy_bpm_preferred_max: f32,

    /// Legacy guardrails: soft BPM range (default: 60–180).
    /// Values in [soft_min, preferred_min) or (preferred_max, soft_max] get a medium cap.
    pub legacy_bpm_soft_min: f32,
    /// Legacy guardrails: soft BPM range upper bound (default: 180).
    pub legacy_bpm_soft_max: f32,

    /// Legacy guardrails: confidence caps by range.
    /// - preferred: inside [preferred_min, preferred_max]
    /// - soft: inside [soft_min, soft_max] but outside preferred
    /// - extreme: outside [soft_min, soft_max]
    ///
    /// **Multiplier semantics**: these are applied as `confidence *= multiplier` to legacy
    /// candidates/estimates (softly biasing the selection).
    pub legacy_bpm_conf_mul_preferred: f32,
    /// Legacy guardrails: confidence multiplier for the soft band (default: 0.50).
    pub legacy_bpm_conf_mul_soft: f32,
    /// Legacy guardrails: confidence multiplier for extremes (default: 0.10).
    pub legacy_bpm_conf_mul_extreme: f32,

    /// Minimum BPM to consider (default: 60.0)
    pub min_bpm: f32,

    /// Maximum BPM to consider (default: 180.0)
    pub max_bpm: f32,

    /// BPM resolution for comb filterbank (default: 1.0)
    pub bpm_resolution: f32,

    // STFT parameters
    /// Frame size for STFT (default: 2048)
    pub frame_size: usize,

    /// Hop size for STFT (default: 512)
    pub hop_size: usize,

    // Key detection
    /// Center frequency for chroma extraction (default: 440.0 Hz, A4)
    pub center_frequency: f32,

    /// Enable soft chroma mapping (default: true)
    /// Soft mapping spreads frequency bins to neighboring semitones for robustness
    pub soft_chroma_mapping: bool,

    /// Soft mapping standard deviation in semitones (default: 0.5)
    /// Lower values = sharper mapping, higher values = more spread
    pub soft_mapping_sigma: f32,

    /// Chroma sharpening power (default: 1.0 = no sharpening, 1.5-2.0 recommended)
    /// Power > 1.0 emphasizes prominent semitones, improving key detection
    pub chroma_sharpening_power: f32,

    /// Enable a lightweight percussive-suppression step for key detection by time-smoothing
    /// the STFT magnitude spectrogram prior to chroma extraction.
    ///
    /// This is HPSS-inspired (harmonic content is sustained in time; percussive is transient),
    /// but uses a cheap moving-average rather than full iterative HPSS.
    ///
    /// Default: true.
    pub enable_key_spectrogram_time_smoothing: bool,

    /// Half-window size (in frames) for the key spectrogram time-smoothing.
    /// Effective window length is `2*margin + 1`.
    ///
    /// Default: 12 (≈ 12 * hop_size samples ≈ 140 ms at 44.1kHz with hop=512).
    pub key_spectrogram_smooth_margin: usize,

    /// Enable weighted key aggregation (frame weights based on tonality + energy).
    /// Default: true.
    pub enable_key_frame_weighting: bool,

    /// Minimum per-frame "tonalness" required to include the frame in key aggregation.
    /// Tonalness is computed from chroma entropy and mapped to [0, 1].
    /// Default: 0.10.
    pub key_min_tonalness: f32,

    /// Exponent applied to tonalness when building frame weights (>= 0).
    /// Default: 2.0.
    pub key_tonalness_power: f32,

    /// Exponent applied to normalized frame energy when building frame weights (>= 0).
    /// Default: 0.50 (square-root weighting).
    pub key_energy_power: f32,

    /// Enable a harmonic-emphasized spectrogram for key detection via a time-smoothing-derived
    /// soft mask (cheap HPSS-inspired).
    ///
    /// If enabled, key detection uses `harmonic_spectrogram_time_mask()` instead of raw/time-smoothed
    /// magnitudes when extracting chroma.
    ///
    /// Default: true.
    pub enable_key_harmonic_mask: bool,

    /// Soft-mask exponent \(p\) for harmonic masking (>= 1.0). Higher values produce harder masks.
    /// Default: 2.0.
    pub key_harmonic_mask_power: f32,

    /// Enable median-filter HPSS harmonic extraction for key detection (key-only).
    ///
    /// This is a more literature-standard HPSS step than `harmonic_spectrogram_time_mask()`.
    /// We compute time- and frequency-median estimates on a **time-downsampled**, **band-limited**
    /// spectrogram, build a soft mask, then apply it to the full-resolution spectrogram.
    ///
    /// Default: false (opt-in; more expensive).
    pub enable_key_hpss_harmonic: bool,

    /// Time-downsampling step for key HPSS (>= 1). Values like 2–6 greatly reduce cost.
    /// Default: 4.
    pub key_hpss_frame_step: usize,

    /// Half-window size (in downsampled frames) for the HPSS harmonic (time) median filter.
    /// Effective window length is `2*margin + 1` (in downsampled frames).
    /// Default: 8.
    pub key_hpss_time_margin: usize,

    /// Half-window size (in frequency bins) for the HPSS percussive (frequency) median filter.
    /// Effective window length is `2*margin + 1` bins.
    /// Default: 8.
    pub key_hpss_freq_margin: usize,

    /// Soft-mask exponent \(p\) for HPSS masking (>= 1.0). Higher values produce harder masks.
    /// Default: 2.0.
    pub key_hpss_mask_power: f32,

    /// Enable a key-only STFT override (compute a separate STFT for key detection).
    ///
    /// Rationale: key detection benefits from higher frequency resolution than BPM/onset work.
    /// A larger FFT size improves pitch precision at low frequencies where semitone spacing is small.
    ///
    /// Default: false (keep single shared STFT by default).
    pub enable_key_stft_override: bool,

    /// FFT frame size used for key-only STFT when `enable_key_stft_override` is true.
    /// Default: 8192.
    pub key_stft_frame_size: usize,

    /// Hop size used for key-only STFT when `enable_key_stft_override` is true.
    /// Default: 512.
    pub key_stft_hop_size: usize,

    /// Enable log-frequency (semitone-aligned) spectrogram for key detection.
    ///
    /// This converts the linear STFT magnitude spectrogram into a log-frequency representation
    /// where each bin corresponds to one semitone. This provides better pitch-class resolution
    /// than mapping linear FFT bins to semitones, especially at low frequencies.
    ///
    /// When enabled, chroma extraction works directly on semitone bins (no frequency-to-semitone
    /// mapping needed). HPCP is disabled when log-frequency is enabled (HPCP requires frequency
    /// information for harmonic summation).
    ///
    /// Default: false (use linear STFT with frequency-to-semitone mapping).
    pub enable_key_log_frequency: bool,

    /// Enable beat-synchronous chroma extraction for key detection.
    ///
    /// This aligns chroma windows to beat boundaries instead of fixed-time frames, improving
    /// harmonic coherence by aligning to musical structure. For each beat interval, chroma vectors
    /// from all STFT frames within that interval are averaged.
    ///
    /// Requires a valid beat grid (falls back to frame-based chroma if beat grid is unavailable).
    /// HPCP is disabled when beat-synchronous is enabled (HPCP requires frame-based processing).
    ///
    /// Default: false (use frame-based chroma extraction).
    pub enable_key_beat_synchronous: bool,

    /// Enable multi-scale key detection (ensemble voting across multiple time scales).
    ///
    /// This runs key detection at multiple segment lengths (short, medium, long) and aggregates
    /// results using clarity-weighted voting. This captures both local and global key information,
    /// improving robustness on tracks with key changes or varying harmonic stability.
    ///
    /// Default: false (use single-scale detection).
    pub enable_key_multi_scale: bool,

    /// Template set to use for key detection.
    ///
    /// - `KrumhanslKessler`: Krumhansl-Kessler (1982) templates (empirical, from listening experiments)
    /// - `Temperley`: Temperley (1999) templates (statistical, from corpus analysis)
    ///
    /// Default: `KrumhanslKessler`.
    pub key_template_set: crate::features::key::templates::TemplateSet,

    /// Enable ensemble key detection (combine K-K and Temperley template scores).
    ///
    /// This runs key detection with both template sets and combines their scores using
    /// weighted voting. This ensemble approach can improve robustness by leveraging
    /// complementary strengths of different template sets.
    ///
    /// Default: false (use single template set).
    pub enable_key_ensemble: bool,

    /// Weight for Krumhansl-Kessler scores in ensemble detection.
    ///
    /// Default: 0.5 (equal weight with Temperley).
    pub key_ensemble_kk_weight: f32,

    /// Weight for Temperley scores in ensemble detection.
    ///
    /// Default: 0.5 (equal weight with K-K).
    pub key_ensemble_temperley_weight: f32,

    /// Enable median key detection (detect key from multiple short segments and select median).
    ///
    /// This divides the track into multiple short overlapping segments, detects key for each
    /// segment, and selects the median key (most common key across segments). This helps
    /// handle brief modulations, breakdowns, or ambiguous sections.
    ///
    /// Default: false (use global key detection).
    pub enable_key_median: bool,

    /// Segment length (in frames) for median key detection.
    ///
    /// Default: 480 (~4 seconds at typical frame rates).
    pub key_median_segment_length_frames: usize,

    /// Segment hop size (in frames) for median key detection.
    ///
    /// Default: 120 (~1 second).
    pub key_median_segment_hop_frames: usize,

    /// Minimum number of segments required for median key detection.
    ///
    /// If fewer segments are available, falls back to global detection.
    ///
    /// Default: 3.
    pub key_median_min_segments: usize,

    /// Segment lengths (in frames) for multi-scale key detection.
    /// Multiple scales are processed and aggregated with clarity-weighted voting.
    /// Default: [120, 360, 720] (approximately 2s, 6s, 12s at typical frame rates).
    pub key_multi_scale_lengths: Vec<usize>,

    /// Hop size (in frames) between segments for multi-scale detection.
    /// Default: 60 (approximately 1s at typical frame rates).
    pub key_multi_scale_hop: usize,

    /// Minimum clarity threshold for including a segment in multi-scale aggregation.
    /// Default: 0.20.
    pub key_multi_scale_min_clarity: f32,

    /// Optional weights for each scale in multi-scale detection (if empty, all scales weighted equally).
    /// Length should match `key_multi_scale_lengths`. Default: empty (equal weights).
    pub key_multi_scale_weights: Vec<f32>,

    /// Enable per-track tuning compensation for key detection.
    ///
    /// This estimates a global detuning offset (in semitones, relative to A4=440Hz) from the
    /// key spectrogram, then shifts semitone mapping by that offset during chroma extraction.
    ///
    /// Default: true.
    pub enable_key_tuning_compensation: bool,

    /// Maximum absolute tuning correction to apply (semitones).
    /// Default: 0.25.
    pub key_tuning_max_abs_semitones: f32,

    /// Frame subsampling step used for tuning estimation (>= 1).
    /// Default: 20.
    pub key_tuning_frame_step: usize,

    /// Relative threshold (fraction of per-frame peak) for selecting bins used in tuning estimation.
    /// Default: 0.35.
    pub key_tuning_peak_rel_threshold: f32,

    /// Enable trimming the first/last fraction of frames for key detection.
    ///
    /// DJ tracks often have long beat-only intros/outros; trimming edges reduces percussive bias
    /// without affecting tempo (tempo uses its own pipeline).
    ///
    /// Default: true.
    pub enable_key_edge_trim: bool,

    /// Fraction (0..0.49) to trim from the start and end (symmetric) when `enable_key_edge_trim` is true.
    /// Default: 0.15 (use middle 70%).
    pub key_edge_trim_fraction: f32,

    /// Enable segment voting for key detection (windowed key detection + score accumulation).
    ///
    /// Rationale: long-form DJ tracks can modulate, have breakdowns, or contain beat-only sections.
    /// Segment voting helps focus on harmonically stable portions without requiring full key-change tracking.
    ///
    /// Default: true.
    pub enable_key_segment_voting: bool,

    /// Segment length in chroma frames for key voting.
    /// Default: 1024 (~11.9s at 44.1kHz, hop=512).
    pub key_segment_len_frames: usize,

    /// Segment hop/stride in frames for key voting.
    /// Default: 512 (~50% overlap).
    pub key_segment_hop_frames: usize,

    /// Minimum clarity required to include a segment in voting (0..1).
    /// Default: 0.20.
    pub key_segment_min_clarity: f32,

    /// Enable a conservative mode heuristic to reduce minor→major mistakes.
    ///
    /// Uses the 3rd degree (minor third vs major third) from the aggregated chroma to potentially
    /// flip parallel mode, gated by a score-ratio threshold.
    ///
    /// Default: true.
    pub enable_key_mode_heuristic: bool,

    /// Required ratio margin for the 3rd-degree test (>=0). If `p(min3) > p(maj3) * (1+margin)`
    /// we prefer minor (and vice versa for major).
    /// Default: 0.05.
    pub key_mode_third_ratio_margin: f32,

    /// Only flip parallel mode if the alternate mode's template score is at least this ratio of
    /// the best mode's score (0..1).
    /// Default: 0.92.
    pub key_mode_flip_min_score_ratio: f32,

    /// Enable HPCP-style pitch-class profile extraction for key detection.
    ///
    /// This uses spectral peak picking + harmonic summation to form a more robust tonal profile
    /// than raw STFT-bin chroma on real-world mixes.
    ///
    /// Default: false (experimental).
    pub enable_key_hpcp: bool,

    /// Number of spectral peaks per frame used for HPCP extraction.
    /// Default: 24.
    pub key_hpcp_peaks_per_frame: usize,

    /// Number of harmonics per peak used for HPCP extraction.
    /// Default: 4.
    pub key_hpcp_num_harmonics: usize,

    /// Harmonic decay factor applied per harmonic (0..1). Lower values emphasize fundamentals.
    /// Default: 0.60.
    pub key_hpcp_harmonic_decay: f32,

    /// Magnitude compression exponent for peak weights (0..1].
    /// Default: 0.50 (sqrt).
    pub key_hpcp_mag_power: f32,

    /// Enable spectral whitening (per-frame frequency-domain normalization) for HPCP peak picking.
    ///
    /// This suppresses timbral formants and broadband coloration, helping peaks corresponding to
    /// harmonic partials stand out more consistently across mixes.
    ///
    /// Default: false.
    pub enable_key_hpcp_whitening: bool,

    /// Frequency smoothing window (in FFT bins) for HPCP whitening.
    /// Larger values whiten more aggressively (more timbre suppression), but can also amplify noise.
    ///
    /// Default: 31.
    pub key_hpcp_whitening_smooth_bins: usize,

    /// Enable a bass-band HPCP blend (tonic reinforcement).
    ///
    /// Relative major/minor share pitch classes; bass/tonic emphasis can disambiguate mode in
    /// dance music where the bassline strongly implies the tonic.
    ///
    /// Default: true.
    pub enable_key_hpcp_bass_blend: bool,

    /// Bass-band lower cutoff (Hz) for bass HPCP.
    /// Default: 55.0.
    pub key_hpcp_bass_fmin_hz: f32,

    /// Bass-band upper cutoff (Hz) for bass HPCP.
    /// Default: 300.0.
    pub key_hpcp_bass_fmax_hz: f32,

    /// Blend weight for bass HPCP (0..1). Final PCP = normalize((1-w)*full + w*bass).
    /// Default: 0.35.
    pub key_hpcp_bass_weight: f32,

    /// Enable a minor-key harmonic bonus (leading-tone vs flat-7) when scoring templates.
    ///
    /// Many dance tracks in minor heavily use harmonic minor gestures (raised 7th). This bonus
    /// nudges minor candidates whose pitch-class distribution supports a leading-tone.
    ///
    /// Default: true.
    pub enable_key_minor_harmonic_bonus: bool,

    /// Weight for the minor harmonic bonus. Internally scaled by the sum of frame weights so it
    /// is comparable to the template-score scale.
    ///
    /// Default: 0.8.
    pub key_minor_leading_tone_bonus_weight: f32,

    // Kick-pattern detector
    /// Lower edge of the kick band in Hz.
    /// Default: 40.0.
    pub kick_pattern_band_low_hz: f32,

    /// Upper edge of the kick band in Hz.
    /// Default: 200.0.
    pub kick_pattern_band_high_hz: f32,

    /// Percentile threshold used by kick-band onset detection.
    /// Default: 0.85.
    pub kick_pattern_onset_threshold_percentile: f32,

    /// Kicks per bar below which the detector reports `Sparse`.
    /// Default: 0.5.
    pub kick_pattern_sparse_threshold: f32,

    /// Minimum template cosine score required before reporting a non-irregular
    /// placement category.
    /// Default: 0.4.
    pub kick_pattern_min_template_score: f32,

    /// Minimum BPM at which a two-kick-per-bar pattern may be reported as
    /// halftime. Below this, it collapses toward straight four-on-floor with
    /// reduced confidence.
    /// Default: 100.0.
    pub kick_pattern_halftime_min_bpm: f32,

    // ML refinement
    /// Enable ML refinement (requires ml feature)
    #[cfg(feature = "ml")]
    pub enable_ml_refinement: bool,

    /// Pre-supplied beat grid (e.g. from Rekordbox ANLZ). When `Some`, the
    /// HMM beat tracker is skipped entirely and this grid is used downstream.
    /// Beats, downbeats, and bars must satisfy [`BeatGrid::validate`].
    /// Default: `None`.
    pub external_beat_grid: Option<BeatGrid>,
}

impl AnalysisConfig {
    /// Validate all enabled analysis paths before samples are cloned or DSP state is allocated.
    pub fn validate(&self, sample_rate: u32, sample_count: usize) -> Result<(), AnalysisError> {
        if sample_rate == 0 {
            return Err(AnalysisError::InvalidInput(
                "sample_rate must be greater than 0, got 0".to_string(),
            ));
        }
        if sample_count == 0 {
            return Err(AnalysisError::InvalidInput(
                "sample_count must be greater than 0, got 0".to_string(),
            ));
        }

        for &(field, value) in &[
            ("min_amplitude_db", self.min_amplitude_db),
            (
                "onset_threshold_percentile",
                self.onset_threshold_percentile,
            ),
            ("tempogram_multi_res_w512", self.tempogram_multi_res_w512),
            ("tempogram_multi_res_w256", self.tempogram_multi_res_w256),
            ("tempogram_multi_res_w1024", self.tempogram_multi_res_w1024),
            (
                "tempogram_multi_res_structural_discount",
                self.tempogram_multi_res_structural_discount,
            ),
            (
                "tempogram_multi_res_double_time_512_factor",
                self.tempogram_multi_res_double_time_512_factor,
            ),
            (
                "tempogram_multi_res_margin_threshold",
                self.tempogram_multi_res_margin_threshold,
            ),
            ("tempogram_band_low_max_hz", self.tempogram_band_low_max_hz),
            ("tempogram_band_mid_max_hz", self.tempogram_band_mid_max_hz),
            (
                "tempogram_band_high_max_hz",
                self.tempogram_band_high_max_hz,
            ),
            ("tempogram_band_w_full", self.tempogram_band_w_full),
            ("tempogram_band_w_low", self.tempogram_band_w_low),
            ("tempogram_band_w_mid", self.tempogram_band_w_mid),
            ("tempogram_band_w_high", self.tempogram_band_w_high),
            (
                "tempogram_band_support_threshold",
                self.tempogram_band_support_threshold,
            ),
            (
                "tempogram_band_consensus_bonus",
                self.tempogram_band_consensus_bonus,
            ),
            (
                "tempogram_novelty_w_spectral",
                self.tempogram_novelty_w_spectral,
            ),
            (
                "tempogram_novelty_w_energy",
                self.tempogram_novelty_w_energy,
            ),
            ("tempogram_novelty_w_hfc", self.tempogram_novelty_w_hfc),
            ("tempogram_mel_fmin_hz", self.tempogram_mel_fmin_hz),
            ("tempogram_mel_fmax_hz", self.tempogram_mel_fmax_hz),
            ("tempogram_mel_weight", self.tempogram_mel_weight),
            ("legacy_bpm_preferred_min", self.legacy_bpm_preferred_min),
            ("legacy_bpm_preferred_max", self.legacy_bpm_preferred_max),
            ("legacy_bpm_soft_min", self.legacy_bpm_soft_min),
            ("legacy_bpm_soft_max", self.legacy_bpm_soft_max),
            (
                "legacy_bpm_conf_mul_preferred",
                self.legacy_bpm_conf_mul_preferred,
            ),
            ("legacy_bpm_conf_mul_soft", self.legacy_bpm_conf_mul_soft),
            (
                "legacy_bpm_conf_mul_extreme",
                self.legacy_bpm_conf_mul_extreme,
            ),
            ("min_bpm", self.min_bpm),
            ("max_bpm", self.max_bpm),
            ("bpm_resolution", self.bpm_resolution),
            ("center_frequency", self.center_frequency),
            ("soft_mapping_sigma", self.soft_mapping_sigma),
            ("chroma_sharpening_power", self.chroma_sharpening_power),
            ("key_min_tonalness", self.key_min_tonalness),
            ("key_tonalness_power", self.key_tonalness_power),
            ("key_energy_power", self.key_energy_power),
            ("key_harmonic_mask_power", self.key_harmonic_mask_power),
            ("key_hpss_mask_power", self.key_hpss_mask_power),
            ("key_ensemble_kk_weight", self.key_ensemble_kk_weight),
            (
                "key_ensemble_temperley_weight",
                self.key_ensemble_temperley_weight,
            ),
            (
                "key_multi_scale_min_clarity",
                self.key_multi_scale_min_clarity,
            ),
            (
                "key_tuning_max_abs_semitones",
                self.key_tuning_max_abs_semitones,
            ),
            (
                "key_tuning_peak_rel_threshold",
                self.key_tuning_peak_rel_threshold,
            ),
            ("key_edge_trim_fraction", self.key_edge_trim_fraction),
            ("key_segment_min_clarity", self.key_segment_min_clarity),
            (
                "key_mode_third_ratio_margin",
                self.key_mode_third_ratio_margin,
            ),
            (
                "key_mode_flip_min_score_ratio",
                self.key_mode_flip_min_score_ratio,
            ),
            ("key_hpcp_harmonic_decay", self.key_hpcp_harmonic_decay),
            ("key_hpcp_mag_power", self.key_hpcp_mag_power),
            ("key_hpcp_bass_fmin_hz", self.key_hpcp_bass_fmin_hz),
            ("key_hpcp_bass_fmax_hz", self.key_hpcp_bass_fmax_hz),
            ("key_hpcp_bass_weight", self.key_hpcp_bass_weight),
            (
                "key_minor_leading_tone_bonus_weight",
                self.key_minor_leading_tone_bonus_weight,
            ),
            ("kick_pattern_band_low_hz", self.kick_pattern_band_low_hz),
            ("kick_pattern_band_high_hz", self.kick_pattern_band_high_hz),
            (
                "kick_pattern_onset_threshold_percentile",
                self.kick_pattern_onset_threshold_percentile,
            ),
            (
                "kick_pattern_sparse_threshold",
                self.kick_pattern_sparse_threshold,
            ),
            (
                "kick_pattern_min_template_score",
                self.kick_pattern_min_template_score,
            ),
            (
                "kick_pattern_halftime_min_bpm",
                self.kick_pattern_halftime_min_bpm,
            ),
        ] {
            require_finite(field, value)?;
        }
        if let Some(debug_gt_bpm) = self.debug_gt_bpm {
            require_finite("debug_gt_bpm", debug_gt_bpm)?;
        }
        for (index, &weight) in self.onset_consensus_weights.iter().enumerate() {
            require_finite(&format!("onset_consensus_weights[{index}]"), weight)?;
        }
        for (index, &weight) in self.key_multi_scale_weights.iter().enumerate() {
            require_finite(&format!("key_multi_scale_weights[{index}]"), weight)?;
        }

        let (_, shared_bins) = validate_spectrogram_request(
            sample_count,
            self.frame_size,
            self.hop_size,
            "frame_size",
            "hop_size",
        )?;
        if self.enable_tempogram_multi_resolution && !self.force_legacy_bpm {
            for hop in [256, 512, 1024] {
                validate_spectrogram_request(
                    sample_count,
                    self.frame_size,
                    hop,
                    "frame_size",
                    "tempogram_multi_resolution_hop",
                )?;
            }
        }

        if self.enable_onset_consensus {
            require_closed_unit(
                "onset_threshold_percentile",
                self.onset_threshold_percentile,
            )?;
            require_weight_group("onset_consensus_weights", &self.onset_consensus_weights)?;
            if self.onset_consensus_tolerance_ms == 0 {
                return Err(AnalysisError::InvalidInput(
                    "onset_consensus_tolerance_ms must be greater than 0, got 0".to_string(),
                ));
            }
        }
        if (self.enable_onset_consensus && self.enable_hpss_onsets)
            || (!self.force_legacy_bpm
                && self.enable_tempogram_multi_resolution
                && self.enable_tempogram_percussive_fallback)
        {
            require_window("hpss_margin", self.hpss_margin)?;
        }

        require_positive("min_bpm", self.min_bpm)?;
        if self.max_bpm <= self.min_bpm {
            return Err(AnalysisError::InvalidInput(format!(
                "max_bpm must be greater than min_bpm, got {} <= {}",
                self.max_bpm, self.min_bpm
            )));
        }
        require_positive("bpm_resolution", self.bpm_resolution)?;
        let bpm_candidates = bpm_candidate_count(self.min_bpm, self.max_bpm, self.bpm_resolution)?;

        if self.enable_tempogram_multi_resolution && !self.force_legacy_bpm {
            if self.tempogram_multi_res_top_k == 0
                || self.tempogram_multi_res_top_k > bpm_candidates
            {
                return Err(AnalysisError::InvalidInput(format!(
                    "tempogram_multi_res_top_k must be in 1..={bpm_candidates}, got {}",
                    self.tempogram_multi_res_top_k
                )));
            }
            require_weight_group(
                "tempogram_multi_resolution_weights",
                &[
                    self.tempogram_multi_res_w512,
                    self.tempogram_multi_res_w256,
                    self.tempogram_multi_res_w1024,
                ],
            )?;
            require_non_negative(
                "tempogram_multi_res_structural_discount",
                self.tempogram_multi_res_structural_discount,
            )?;
            require_non_negative(
                "tempogram_multi_res_double_time_512_factor",
                self.tempogram_multi_res_double_time_512_factor,
            )?;
            require_non_negative(
                "tempogram_multi_res_margin_threshold",
                self.tempogram_multi_res_margin_threshold,
            )?;
        }
        if self.emit_tempogram_candidates
            && (self.tempogram_candidates_top_n == 0
                || self.tempogram_candidates_top_n > bpm_candidates)
        {
            return Err(AnalysisError::InvalidInput(format!(
                "tempogram_candidates_top_n must be in 1..={bpm_candidates}, got {}",
                self.tempogram_candidates_top_n
            )));
        }
        if self.debug_track_id.is_some()
            && (self.debug_top_n == 0 || self.debug_top_n > bpm_candidates)
        {
            return Err(AnalysisError::InvalidInput(format!(
                "debug_top_n must be in 1..={bpm_candidates}, got {}",
                self.debug_top_n
            )));
        }

        let nyquist = sample_rate as f32 / 2.0;
        if self.enable_tempogram_band_fusion && !self.force_legacy_bpm {
            let high = if self.tempogram_band_high_max_hz <= 0.0 {
                nyquist
            } else {
                self.tempogram_band_high_max_hz
            };
            if !(0.0 < self.tempogram_band_low_max_hz
                && self.tempogram_band_low_max_hz < self.tempogram_band_mid_max_hz
                && self.tempogram_band_mid_max_hz < high
                && high <= nyquist)
            {
                return Err(AnalysisError::InvalidInput(format!(
                    "tempogram_band_*_hz must satisfy 0 < low < mid < high <= Nyquist ({nyquist}), got low={}, mid={}, high={}",
                    self.tempogram_band_low_max_hz, self.tempogram_band_mid_max_hz, high
                )));
            }
            require_weight_group(
                "tempogram_band_weights",
                &[
                    self.tempogram_band_w_full,
                    self.tempogram_band_w_low,
                    self.tempogram_band_w_mid,
                    self.tempogram_band_w_high,
                ],
            )?;
            require_closed_unit(
                "tempogram_band_support_threshold",
                self.tempogram_band_support_threshold,
            )?;
            require_non_negative(
                "tempogram_band_consensus_bonus",
                self.tempogram_band_consensus_bonus,
            )?;
        }

        let use_tempogram_aux = !self.force_legacy_bpm
            && (self.enable_tempogram_band_fusion
                || self.enable_tempogram_mel_novelty
                || self.tempogram_band_consensus_bonus > 0.0);
        if use_tempogram_aux {
            require_non_negative(
                "tempogram_band_consensus_bonus",
                self.tempogram_band_consensus_bonus,
            )?;
            require_weight_group(
                "tempogram_novelty_weights",
                &[
                    self.tempogram_novelty_w_spectral,
                    self.tempogram_novelty_w_energy,
                    self.tempogram_novelty_w_hfc,
                ],
            )?;
            require_window(
                "tempogram_superflux_max_filter_bins",
                self.tempogram_superflux_max_filter_bins,
            )?;
        }

        if self.enable_tempogram_mel_novelty && !self.force_legacy_bpm {
            if self.tempogram_mel_n_mels == 0 || self.tempogram_mel_n_mels > shared_bins {
                return Err(AnalysisError::InvalidInput(format!(
                    "tempogram_mel_n_mels must be in 1..={shared_bins}, got {}",
                    self.tempogram_mel_n_mels
                )));
            }
            let mel_high = if self.tempogram_mel_fmax_hz <= 0.0 {
                nyquist
            } else {
                self.tempogram_mel_fmax_hz
            };
            if !(0.0 < self.tempogram_mel_fmin_hz
                && self.tempogram_mel_fmin_hz < mel_high
                && mel_high <= nyquist)
            {
                return Err(AnalysisError::InvalidInput(format!(
                    "tempogram_mel_fmin_hz/tempogram_mel_fmax_hz must satisfy 0 < min < max <= Nyquist ({nyquist}), got min={}, max={mel_high}",
                    self.tempogram_mel_fmin_hz
                )));
            }
            require_window(
                "tempogram_mel_max_filter_bins",
                self.tempogram_mel_max_filter_bins,
            )?;
            require_non_negative("tempogram_mel_weight", self.tempogram_mel_weight)?;
        }

        if self.enable_legacy_bpm_guardrails {
            if !(0.0 < self.legacy_bpm_soft_min
                && self.legacy_bpm_soft_min <= self.legacy_bpm_preferred_min
                && self.legacy_bpm_preferred_min < self.legacy_bpm_preferred_max
                && self.legacy_bpm_preferred_max <= self.legacy_bpm_soft_max)
            {
                return Err(AnalysisError::InvalidInput(format!(
                    "legacy BPM ranges must satisfy 0 < soft_min <= preferred_min < preferred_max <= soft_max, got {}, {}, {}, {}",
                    self.legacy_bpm_soft_min,
                    self.legacy_bpm_preferred_min,
                    self.legacy_bpm_preferred_max,
                    self.legacy_bpm_soft_max
                )));
            }
            for &(field, value) in &[
                (
                    "legacy_bpm_conf_mul_preferred",
                    self.legacy_bpm_conf_mul_preferred,
                ),
                ("legacy_bpm_conf_mul_soft", self.legacy_bpm_conf_mul_soft),
                (
                    "legacy_bpm_conf_mul_extreme",
                    self.legacy_bpm_conf_mul_extreme,
                ),
            ] {
                require_non_negative(field, value)?;
            }
        }

        if !self.enable_key_log_frequency
            && (self.soft_chroma_mapping
                || (!self.enable_key_beat_synchronous && self.enable_key_hpcp))
        {
            require_positive("soft_mapping_sigma", self.soft_mapping_sigma)?;
        }
        require_non_negative("chroma_sharpening_power", self.chroma_sharpening_power)?;

        if !self.enable_key_hpss_harmonic
            && (self.enable_key_spectrogram_time_smoothing || self.enable_key_harmonic_mask)
        {
            require_window(
                "key_spectrogram_smooth_margin",
                self.key_spectrogram_smooth_margin,
            )?;
        }
        if self.enable_key_frame_weighting {
            require_closed_unit("key_min_tonalness", self.key_min_tonalness)?;
            require_non_negative("key_tonalness_power", self.key_tonalness_power)?;
            require_non_negative("key_energy_power", self.key_energy_power)?;
        }
        if !self.enable_key_hpss_harmonic
            && self.enable_key_harmonic_mask
            && self.key_harmonic_mask_power < 1.0
        {
            return Err(AnalysisError::InvalidInput(format!(
                "key_harmonic_mask_power must be at least 1, got {}",
                self.key_harmonic_mask_power
            )));
        }

        let key_bins = if self.enable_key_stft_override {
            let (_, bins) = validate_spectrogram_request(
                sample_count,
                self.key_stft_frame_size,
                self.key_stft_hop_size,
                "key_stft_frame_size",
                "key_stft_hop_size",
            )?;
            bins
        } else {
            shared_bins
        };
        if self.enable_key_hpss_harmonic {
            if self.key_hpss_frame_step == 0 {
                return Err(AnalysisError::InvalidInput(
                    "key_hpss_frame_step must be greater than 0, got 0".to_string(),
                ));
            }
            require_window("key_hpss_time_margin", self.key_hpss_time_margin)?;
            require_window("key_hpss_freq_margin", self.key_hpss_freq_margin)?;
            if self.key_hpss_mask_power < 1.0 {
                return Err(AnalysisError::InvalidInput(format!(
                    "key_hpss_mask_power must be at least 1, got {}",
                    self.key_hpss_mask_power
                )));
            }
            if nyquist < 5_000.0 {
                return Err(AnalysisError::InvalidInput(format!(
                    "enable_key_hpss_harmonic requires Nyquist >= 5000 Hz, got {nyquist}"
                )));
            }
        }
        if self.enable_key_log_frequency && nyquist < 5_000.0 {
            return Err(AnalysisError::InvalidInput(format!(
                "enable_key_log_frequency requires Nyquist >= 5000 Hz, got {nyquist}"
            )));
        }

        if self.enable_key_ensemble {
            require_weight_group(
                "key_ensemble_weights",
                &[
                    self.key_ensemble_kk_weight,
                    self.key_ensemble_temperley_weight,
                ],
            )?;
        }
        if !self.enable_key_ensemble && self.enable_key_multi_scale {
            if self.key_multi_scale_lengths.is_empty() || self.key_multi_scale_lengths.contains(&0)
            {
                return Err(AnalysisError::InvalidInput(
                    "key_multi_scale_lengths must be non-empty and contain only positive lengths"
                        .to_string(),
                ));
            }
            if self.key_multi_scale_hop == 0 {
                return Err(AnalysisError::InvalidInput(
                    "key_multi_scale_hop must be greater than 0, got 0".to_string(),
                ));
            }
            require_closed_unit(
                "key_multi_scale_min_clarity",
                self.key_multi_scale_min_clarity,
            )?;
            if !self.key_multi_scale_weights.is_empty() {
                if self.key_multi_scale_weights.len() != self.key_multi_scale_lengths.len() {
                    return Err(AnalysisError::InvalidInput(format!(
                        "key_multi_scale_weights length must match key_multi_scale_lengths, got {} and {}",
                        self.key_multi_scale_weights.len(),
                        self.key_multi_scale_lengths.len()
                    )));
                }
                require_weight_group("key_multi_scale_weights", &self.key_multi_scale_weights)?;
            }
        }
        if self.enable_key_tuning_compensation && !self.enable_key_log_frequency {
            require_non_negative(
                "key_tuning_max_abs_semitones",
                self.key_tuning_max_abs_semitones,
            )?;
            if self.key_tuning_frame_step == 0 {
                return Err(AnalysisError::InvalidInput(
                    "key_tuning_frame_step must be greater than 0, got 0".to_string(),
                ));
            }
            require_closed_unit(
                "key_tuning_peak_rel_threshold",
                self.key_tuning_peak_rel_threshold,
            )?;
        }
        if self.enable_key_edge_trim && !(0.0..=0.49).contains(&self.key_edge_trim_fraction) {
            return Err(AnalysisError::InvalidInput(format!(
                "key_edge_trim_fraction must be in [0, 0.49], got {}",
                self.key_edge_trim_fraction
            )));
        }
        if !self.enable_key_ensemble && self.enable_key_segment_voting {
            if self.key_segment_len_frames == 0 || self.key_segment_hop_frames == 0 {
                return Err(AnalysisError::InvalidInput(format!(
                    "enabled key segment length/hop must be positive, got length={}, hop={}",
                    self.key_segment_len_frames, self.key_segment_hop_frames
                )));
            }
            require_closed_unit("key_segment_min_clarity", self.key_segment_min_clarity)?;
        }
        if !self.enable_key_ensemble && self.enable_key_mode_heuristic {
            require_non_negative(
                "key_mode_third_ratio_margin",
                self.key_mode_third_ratio_margin,
            )?;
            require_closed_unit(
                "key_mode_flip_min_score_ratio",
                self.key_mode_flip_min_score_ratio,
            )?;
        }

        if self.enable_key_hpcp
            && !self.enable_key_log_frequency
            && !self.enable_key_beat_synchronous
        {
            if self.key_hpcp_peaks_per_frame == 0 || self.key_hpcp_peaks_per_frame > key_bins {
                return Err(AnalysisError::InvalidInput(format!(
                    "key_hpcp_peaks_per_frame must be in 1..={key_bins}, got {}",
                    self.key_hpcp_peaks_per_frame
                )));
            }
            if self.key_hpcp_num_harmonics == 0 {
                return Err(AnalysisError::InvalidInput(
                    "key_hpcp_num_harmonics must be greater than 0, got 0".to_string(),
                ));
            }
            require_closed_unit("key_hpcp_harmonic_decay", self.key_hpcp_harmonic_decay)?;
            if !(0.0..=1.0).contains(&self.key_hpcp_mag_power) || self.key_hpcp_mag_power == 0.0 {
                return Err(AnalysisError::InvalidInput(format!(
                    "key_hpcp_mag_power must be in (0, 1], got {}",
                    self.key_hpcp_mag_power
                )));
            }
            if self.enable_key_hpcp_whitening {
                if self.key_hpcp_whitening_smooth_bins == 0 {
                    return Err(AnalysisError::InvalidInput(
                        "key_hpcp_whitening_smooth_bins must be greater than 0, got 0".to_string(),
                    ));
                }
                require_window(
                    "key_hpcp_whitening_smooth_bins",
                    self.key_hpcp_whitening_smooth_bins,
                )?;
            }
            if self.enable_key_hpcp_bass_blend {
                if !(0.0 < self.key_hpcp_bass_fmin_hz
                    && self.key_hpcp_bass_fmin_hz < self.key_hpcp_bass_fmax_hz
                    && self.key_hpcp_bass_fmax_hz <= nyquist)
                {
                    return Err(AnalysisError::InvalidInput(format!(
                        "key_hpcp_bass_fmin_hz/key_hpcp_bass_fmax_hz must satisfy 0 < min < max <= Nyquist ({nyquist}), got {}, {}",
                        self.key_hpcp_bass_fmin_hz, self.key_hpcp_bass_fmax_hz
                    )));
                }
                require_closed_unit("key_hpcp_bass_weight", self.key_hpcp_bass_weight)?;
            }
        }
        if !self.enable_key_ensemble && self.enable_key_minor_harmonic_bonus {
            require_non_negative(
                "key_minor_leading_tone_bonus_weight",
                self.key_minor_leading_tone_bonus_weight,
            )?;
        }

        if !(0.0 < self.kick_pattern_band_low_hz
            && self.kick_pattern_band_low_hz < self.kick_pattern_band_high_hz
            && self.kick_pattern_band_high_hz <= nyquist)
        {
            return Err(AnalysisError::InvalidInput(format!(
                "kick_pattern_band_low_hz/kick_pattern_band_high_hz must satisfy 0 < low < high <= Nyquist ({nyquist}), got {}, {}",
                self.kick_pattern_band_low_hz, self.kick_pattern_band_high_hz
            )));
        }
        require_closed_unit(
            "kick_pattern_onset_threshold_percentile",
            self.kick_pattern_onset_threshold_percentile,
        )?;
        require_non_negative(
            "kick_pattern_sparse_threshold",
            self.kick_pattern_sparse_threshold,
        )?;
        require_closed_unit(
            "kick_pattern_min_template_score",
            self.kick_pattern_min_template_score,
        )?;
        require_positive(
            "kick_pattern_halftime_min_bpm",
            self.kick_pattern_halftime_min_bpm,
        )?;

        if let Some(external_beat_grid) = &self.external_beat_grid {
            external_beat_grid.validate()?;
        }

        Ok(())
    }
}

impl Default for AnalysisConfig {
    fn default() -> Self {
        Self {
            min_amplitude_db: -40.0,
            normalization: NormalizationMethod::Peak,
            enable_normalization: true,
            enable_silence_trimming: true,
            enable_onset_consensus: true,
            onset_threshold_percentile: 0.80,
            onset_consensus_tolerance_ms: 50,
            onset_consensus_weights: [0.25, 0.25, 0.25, 0.25],
            enable_hpss_onsets: false,
            hpss_margin: 10,
            force_legacy_bpm: false,
            enable_bpm_fusion: false,
            enable_legacy_bpm_guardrails: true,
            enable_tempogram_multi_resolution: true,
            tempogram_multi_res_top_k: 25,
            tempogram_multi_res_w512: 0.45,
            tempogram_multi_res_w256: 0.35,
            tempogram_multi_res_w1024: 0.20,
            tempogram_multi_res_structural_discount: 0.85,
            tempogram_multi_res_double_time_512_factor: 0.92,
            tempogram_multi_res_margin_threshold: 0.08,
            tempogram_multi_res_use_human_prior: false,
            // HPSS percussive fallback is very expensive and (so far) has not shown consistent gains.
            // Keep it opt-in to avoid multi-second outliers during batch runs.
            enable_tempogram_percussive_fallback: false,
            enable_tempogram_band_fusion: true,
            // Default cutoffs (Hz): ~kick/bass fundamentals, then body/rhythm textures, then attacks.
            tempogram_band_low_max_hz: 200.0,
            tempogram_band_mid_max_hz: 2000.0,
            tempogram_band_high_max_hz: 8000.0,
            // Default weights: keep full-band as anchor, but allow bands to pull candidates into view.
            tempogram_band_w_full: 0.40,
            tempogram_band_w_low: 0.25,
            tempogram_band_w_mid: 0.20,
            tempogram_band_w_high: 0.15,
            tempogram_band_seed_only: true,
            tempogram_band_support_threshold: 0.25,
            tempogram_band_consensus_bonus: 0.08,
            // Novelty weighting defaults (tuned on 200-track validation):
            // shift weight toward transient-heavy signals (energy/HFC) to reduce octave/subdivision traps.
            tempogram_novelty_w_spectral: 0.30,
            tempogram_novelty_w_energy: 0.35,
            tempogram_novelty_w_hfc: 0.35,
            tempogram_novelty_local_mean_window: 16,
            tempogram_novelty_smooth_window: 5,
            debug_track_id: None,
            debug_gt_bpm: None,
            debug_top_n: 5,
            enable_tempogram_mel_novelty: true,
            tempogram_mel_n_mels: 40,
            tempogram_mel_fmin_hz: 30.0,
            tempogram_mel_fmax_hz: 8000.0,
            tempogram_mel_max_filter_bins: 2,
            tempogram_mel_weight: 0.15,
            tempogram_superflux_max_filter_bins: 4,
            emit_tempogram_candidates: false,
            tempogram_candidates_top_n: 10,
            // Tuned defaults (empirical, small-batch): slightly wider preferred band and
            // slightly less aggressive down-weighting while keeping a strong extreme penalty.
            legacy_bpm_preferred_min: 72.0,
            legacy_bpm_preferred_max: 168.0,
            legacy_bpm_soft_min: 60.0,
            legacy_bpm_soft_max: 210.0,
            legacy_bpm_conf_mul_preferred: 1.30,
            legacy_bpm_conf_mul_soft: 0.70,
            legacy_bpm_conf_mul_extreme: 0.01,
            min_bpm: 40.0, // Lowered from 60.0 to catch slower tracks (ballads, ambient, etc.)
            max_bpm: 240.0, // Raised from 180.0 to catch high-tempo tracks (drum & bass, etc.)
            bpm_resolution: 1.0,
            frame_size: 2048,
            hop_size: 512,
            center_frequency: 440.0,
            soft_chroma_mapping: true,
            soft_mapping_sigma: 0.5,
            chroma_sharpening_power: 1.0, // No sharpening by default (can be enabled with 1.5-2.0)
            enable_key_spectrogram_time_smoothing: true,
            key_spectrogram_smooth_margin: 12,
            enable_key_frame_weighting: true,
            // Default: do not hard-gate frames by tonalness; use soft weighting instead.
            key_min_tonalness: 0.0,
            key_tonalness_power: 2.0,
            key_energy_power: 0.50,
            enable_key_harmonic_mask: true,
            key_harmonic_mask_power: 2.0,
            // Default: off. HPSS median filtering is more expensive than the cheap harmonic mask.
            // Enable via CLI/validation when experimenting.
            enable_key_hpss_harmonic: false,
            key_hpss_frame_step: 4,
            key_hpss_time_margin: 8,
            key_hpss_freq_margin: 8,
            key_hpss_mask_power: 2.0,
            enable_key_stft_override: true,
            key_stft_frame_size: 8192,
            key_stft_hop_size: 512,
            enable_key_log_frequency: false,
            enable_key_beat_synchronous: false,
            enable_key_multi_scale: false,
            key_multi_scale_lengths: vec![120, 360, 720], // ~2s, 6s, 12s at typical frame rates
            key_multi_scale_hop: 60,                      // ~1s
            key_multi_scale_min_clarity: 0.20,
            key_multi_scale_weights: vec![], // Equal weights by default
            key_template_set: TemplateSet::KrumhanslKessler,
            enable_key_ensemble: false,
            key_ensemble_kk_weight: 0.5,
            key_ensemble_temperley_weight: 0.5,
            enable_key_median: false,
            key_median_segment_length_frames: 480, // ~4 seconds at typical frame rates
            key_median_segment_hop_frames: 120,    // ~1 second
            key_median_min_segments: 3,
            // Default: off. Tuning estimation can be unstable on real-world mixes without a more
            // peak/partial-aware frontend (HPCP/CQT). Keep available for experimentation.
            enable_key_tuning_compensation: false,
            key_tuning_max_abs_semitones: 0.08,
            key_tuning_frame_step: 20,
            key_tuning_peak_rel_threshold: 0.35,
            // Default: off. Hard edge trimming can remove useful harmonic content on some tracks.
            // Prefer harmonic masking + frame weighting; keep edge-trim available for experimentation.
            enable_key_edge_trim: false,
            key_edge_trim_fraction: 0.15,
            enable_key_segment_voting: true,
            key_segment_len_frames: 1024,
            key_segment_hop_frames: 512,
            key_segment_min_clarity: 0.20,
            enable_key_mode_heuristic: false,
            // NOTE: Aggressive defaults for Phase 1F DJ validation: minor keys were frequently
            // predicted as major. Keep these tunable via CLI/validation.
            key_mode_third_ratio_margin: 0.00,
            key_mode_flip_min_score_ratio: 0.60,
            enable_key_hpcp: true,
            key_hpcp_peaks_per_frame: 24,
            key_hpcp_num_harmonics: 4,
            key_hpcp_harmonic_decay: 0.60,
            key_hpcp_mag_power: 0.50,
            enable_key_hpcp_whitening: false,
            key_hpcp_whitening_smooth_bins: 31,
            // Experimental: tonic reinforcement can backfire if the bass is not stably pitched.
            enable_key_hpcp_bass_blend: false,
            key_hpcp_bass_fmin_hz: 55.0,
            key_hpcp_bass_fmax_hz: 300.0,
            key_hpcp_bass_weight: 0.35,
            // Experimental: can easily over-bias the result on real-world mixes.
            enable_key_minor_harmonic_bonus: false,
            key_minor_leading_tone_bonus_weight: 0.2,
            kick_pattern_band_low_hz: 40.0,
            kick_pattern_band_high_hz: 200.0,
            kick_pattern_onset_threshold_percentile: 0.85,
            kick_pattern_sparse_threshold: 0.5,
            kick_pattern_min_template_score: 0.4,
            kick_pattern_halftime_min_bpm: 100.0,
            #[cfg(feature = "ml")]
            enable_ml_refinement: false,
            external_beat_grid: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{analyze_audio, AnalysisError};

    const SAMPLE_RATE: u32 = 44_100;
    const SAMPLE_COUNT: usize = SAMPLE_RATE as usize;
    type ConfigMutator = fn(&mut AnalysisConfig);
    type InvalidCase = (&'static str, &'static str, ConfigMutator);

    fn assert_invalid(name: &str, expected_field: &str, mutate: fn(&mut AnalysisConfig)) {
        let mut config = AnalysisConfig::default();
        mutate(&mut config);
        match config.validate(SAMPLE_RATE, SAMPLE_COUNT) {
            Err(AnalysisError::InvalidInput(message)) => assert!(
                message.contains(expected_field),
                "{name}: expected error containing {expected_field:?}, got {message:?}"
            ),
            result => panic!("{name}: expected InvalidInput, got {result:?}"),
        }
    }

    #[test]
    fn analysis_config_default_satisfies_each_supported_rate_and_long_form_input() {
        for sample_rate in [44_100_u32, 48_000, 96_000] {
            let one_second = sample_rate as usize;
            AnalysisConfig::default()
                .validate(sample_rate, one_second)
                .unwrap();

            let four_hours = one_second * 60 * 60 * 4;
            AnalysisConfig::default()
                .validate(sample_rate, four_hours)
                .unwrap();
        }
    }

    #[test]
    fn analysis_config_rejects_invalid_numeric_matrix() {
        let cases: &[InvalidCase] = &[
            ("frame zero", "frame_size", |c| c.frame_size = 0),
            ("frame one", "frame_size", |c| c.frame_size = 1),
            ("hop zero", "hop_size", |c| c.hop_size = 0),
            ("key frame zero", "key_stft_frame_size", |c| {
                c.key_stft_frame_size = 0
            }),
            ("key frame one", "key_stft_frame_size", |c| {
                c.key_stft_frame_size = 1
            }),
            ("key hop zero", "key_stft_hop_size", |c| {
                c.key_stft_hop_size = 0
            }),
            ("non-finite threshold", "min_amplitude_db", |c| {
                c.min_amplitude_db = f32::NAN
            }),
            ("non-finite weight", "tempogram_band_w_low", |c| {
                c.tempogram_band_w_low = f32::INFINITY
            }),
            ("non-finite frequency", "tempogram_band_low_max_hz", |c| {
                c.tempogram_band_low_max_hz = f32::NEG_INFINITY
            }),
            ("non-finite power", "key_harmonic_mask_power", |c| {
                c.key_harmonic_mask_power = f32::NAN
            }),
            ("non-finite BPM", "min_bpm", |c| c.min_bpm = f32::NAN),
            ("non-finite debug BPM", "debug_gt_bpm", |c| {
                c.debug_gt_bpm = Some(f32::INFINITY)
            }),
            ("minimum BPM zero", "min_bpm", |c| c.min_bpm = 0.0),
            ("inverted BPM range", "max_bpm", |c| c.max_bpm = c.min_bpm),
            ("BPM resolution zero", "bpm_resolution", |c| {
                c.bpm_resolution = 0.0
            }),
            (
                "onset percentile below range",
                "onset_threshold_percentile",
                |c| c.onset_threshold_percentile = -0.1,
            ),
            (
                "band support above range",
                "tempogram_band_support_threshold",
                |c| c.tempogram_band_support_threshold = 1.1,
            ),
            ("key tonalness above range", "key_min_tonalness", |c| {
                c.key_min_tonalness = 1.1
            }),
            (
                "segment clarity below range",
                "key_segment_min_clarity",
                |c| c.key_segment_min_clarity = -0.1,
            ),
            ("HPCP decay above range", "key_hpcp_harmonic_decay", |c| {
                c.key_hpcp_harmonic_decay = 1.1
            }),
            (
                "kick percentile above range",
                "kick_pattern_onset_threshold_percentile",
                |c| c.kick_pattern_onset_threshold_percentile = 1.1,
            ),
            (
                "negative consensus weight",
                "onset_consensus_weights",
                |c| c.onset_consensus_weights[0] = -0.1,
            ),
            ("zero consensus weights", "onset_consensus_weights", |c| {
                c.onset_consensus_weights = [0.0; 4]
            }),
            (
                "zero multi-resolution weights",
                "tempogram_multi_resolution_weights",
                |c| {
                    c.tempogram_multi_res_w512 = 0.0;
                    c.tempogram_multi_res_w256 = 0.0;
                    c.tempogram_multi_res_w1024 = 0.0;
                },
            ),
            (
                "negative novelty weight",
                "tempogram_novelty_weights",
                |c| c.tempogram_novelty_w_energy = -0.1,
            ),
            ("zero band weights", "tempogram_band_weights", |c| {
                c.tempogram_band_w_full = 0.0;
                c.tempogram_band_w_low = 0.0;
                c.tempogram_band_w_mid = 0.0;
                c.tempogram_band_w_high = 0.0;
            }),
            ("zero ensemble weights", "key_ensemble_weights", |c| {
                c.enable_key_ensemble = true;
                c.key_ensemble_kk_weight = 0.0;
                c.key_ensemble_temperley_weight = 0.0;
            }),
            ("negative tonalness power", "key_tonalness_power", |c| {
                c.key_tonalness_power = -1.0
            }),
            (
                "harmonic mask power below one",
                "key_harmonic_mask_power",
                |c| c.key_harmonic_mask_power = 0.5,
            ),
            ("unordered tempogram bands", "tempogram_band", |c| {
                c.tempogram_band_low_max_hz = c.tempogram_band_mid_max_hz
            }),
            ("tempogram band above Nyquist", "tempogram_band", |c| {
                c.tempogram_band_high_max_hz = 30_000.0
            }),
            ("unordered mel band", "tempogram_mel_fmin_hz", |c| {
                c.tempogram_mel_fmin_hz = c.tempogram_mel_fmax_hz
            }),
            ("kick band above Nyquist", "kick_pattern_band", |c| {
                c.kick_pattern_band_high_hz = 30_000.0
            }),
            ("bass band above Nyquist", "key_hpcp_bass", |c| {
                c.enable_key_hpcp_bass_blend = true;
                c.key_hpcp_bass_fmax_hz = 30_000.0;
            }),
            (
                "zero onset tolerance",
                "onset_consensus_tolerance_ms",
                |c| c.onset_consensus_tolerance_ms = 0,
            ),
            ("zero key HPSS frame step", "key_hpss_frame_step", |c| {
                c.enable_key_hpss_harmonic = true;
                c.key_hpss_frame_step = 0;
            }),
            ("zero multi-scale hop", "key_multi_scale_hop", |c| {
                c.enable_key_multi_scale = true;
                c.key_multi_scale_hop = 0;
            }),
            (
                "empty multi-scale lengths",
                "key_multi_scale_lengths",
                |c| {
                    c.enable_key_multi_scale = true;
                    c.key_multi_scale_lengths.clear();
                },
            ),
            (
                "mismatched multi-scale weights",
                "key_multi_scale_weights",
                |c| {
                    c.enable_key_multi_scale = true;
                    c.key_multi_scale_weights = vec![1.0];
                },
            ),
            ("zero segment hop", "key segment length/hop", |c| {
                c.key_segment_hop_frames = 0
            }),
            ("zero tuning hop", "key_tuning_frame_step", |c| {
                c.enable_key_tuning_compensation = true;
                c.key_tuning_frame_step = 0;
            }),
            ("HPSS window overflow", "hpss_margin", |c| {
                c.enable_hpss_onsets = true;
                c.hpss_margin = usize::MAX;
            }),
            (
                "key smoothing window overflow",
                "key_spectrogram_smooth_margin",
                |c| c.key_spectrogram_smooth_margin = usize::MAX,
            ),
            (
                "key HPSS time window overflow",
                "key_hpss_time_margin",
                |c| {
                    c.enable_key_hpss_harmonic = true;
                    c.key_hpss_time_margin = usize::MAX;
                },
            ),
            (
                "key HPSS frequency window overflow",
                "key_hpss_freq_margin",
                |c| {
                    c.enable_key_hpss_harmonic = true;
                    c.key_hpss_freq_margin = usize::MAX;
                },
            ),
            (
                "SuperFlux window overflow",
                "tempogram_superflux_max_filter_bins",
                |c| c.tempogram_superflux_max_filter_bins = usize::MAX,
            ),
            (
                "mel window overflow",
                "tempogram_mel_max_filter_bins",
                |c| c.tempogram_mel_max_filter_bins = usize::MAX,
            ),
            (
                "whitening window overflow",
                "key_hpcp_whitening_smooth_bins",
                |c| {
                    c.enable_key_hpcp_whitening = true;
                    c.key_hpcp_whitening_smooth_bins = usize::MAX;
                },
            ),
            ("mel bands exceed FFT bins", "tempogram_mel_n_mels", |c| {
                c.tempogram_mel_n_mels = c.frame_size / 2 + 2
            }),
            (
                "tempogram candidate count exceeds range",
                "tempogram_multi_res_top_k",
                |c| c.tempogram_multi_res_top_k = 202,
            ),
            (
                "HPCP peaks exceed key FFT bins",
                "key_hpcp_peaks_per_frame",
                |c| c.key_hpcp_peaks_per_frame = c.key_stft_frame_size / 2 + 2,
            ),
            (
                "BPM candidate arithmetic overflow",
                "candidate count",
                |c| {
                    c.min_bpm = f32::MIN_POSITIVE;
                    c.max_bpm = f32::MAX;
                    c.bpm_resolution = f32::MIN_POSITIVE;
                },
            ),
            ("spectrogram budget", "frame_size/hop_size", |c| {
                c.frame_size = 512;
                c.hop_size = 1;
            }),
            ("edge trim below range", "key_edge_trim_fraction", |c| {
                c.enable_key_edge_trim = true;
                c.key_edge_trim_fraction = -0.01;
            }),
            ("edge trim above range", "key_edge_trim_fraction", |c| {
                c.enable_key_edge_trim = true;
                c.key_edge_trim_fraction = 0.491;
            }),
        ];

        for &(name, field, mutate) in cases {
            assert_invalid(name, field, mutate);
        }
    }

    #[test]
    fn analysis_config_rejects_malformed_external_beat_grids() {
        let cases = [
            BeatGrid {
                beats: vec![f32::NAN],
                downbeats: vec![],
                bars: vec![],
            },
            BeatGrid {
                beats: vec![0.0, 0.0],
                downbeats: vec![],
                bars: vec![],
            },
            BeatGrid {
                beats: vec![1.0, 0.0],
                downbeats: vec![],
                bars: vec![],
            },
            BeatGrid {
                beats: vec![1.0, 2.0],
                downbeats: vec![0.0],
                bars: vec![0.0],
            },
            BeatGrid {
                beats: vec![0.0, 1.0, 2.0],
                downbeats: vec![1.01],
                bars: vec![1.01],
            },
        ];

        for grid in cases {
            let config = AnalysisConfig {
                external_beat_grid: Some(grid),
                ..AnalysisConfig::default()
            };
            assert!(matches!(
                config.validate(SAMPLE_RATE, SAMPLE_COUNT),
                Err(AnalysisError::InvalidInput(_))
            ));
        }
    }

    #[test]
    fn analysis_config_accepts_clamped_windows_boundaries_and_unused_fields() {
        let valid_grid = BeatGrid {
            beats: vec![0.0, 0.5, 1.0],
            downbeats: vec![0.0, 1.0],
            bars: vec![0.0, 1.0],
        };
        let mut config = AnalysisConfig {
            external_beat_grid: Some(valid_grid),
            enable_hpss_onsets: true,
            hpss_margin: 1_000_000,
            enable_key_edge_trim: true,
            key_edge_trim_fraction: 0.49,
            ..AnalysisConfig::default()
        };
        config.validate(SAMPLE_RATE, SAMPLE_COUNT).unwrap();
        config.key_edge_trim_fraction = 0.0;
        config.validate(SAMPLE_RATE, SAMPLE_COUNT).unwrap();

        config.enable_onset_consensus = false;
        config.enable_tempogram_multi_resolution = false;
        config.enable_tempogram_percussive_fallback = false;
        config.hpss_margin = usize::MAX;
        config.enable_tempogram_band_fusion = false;
        config.enable_tempogram_mel_novelty = false;
        config.tempogram_band_consensus_bonus = 0.0;
        config.tempogram_novelty_w_spectral = -1.0;
        config.tempogram_novelty_w_energy = 0.0;
        config.tempogram_novelty_w_hfc = 0.0;
        config.tempogram_novelty_local_mean_window = 0;
        config.tempogram_superflux_max_filter_bins = usize::MAX;
        config.enable_key_hpss_harmonic = false;
        config.key_hpss_time_margin = usize::MAX;
        config.key_hpss_freq_margin = usize::MAX;
        config.enable_key_multi_scale = false;
        config.key_multi_scale_lengths.clear();
        config.key_multi_scale_hop = 0;
        config.key_multi_scale_weights = vec![-1.0];
        config.enable_key_tuning_compensation = false;
        config.key_tuning_frame_step = 0;
        config.key_tuning_peak_rel_threshold = 2.0;
        config.enable_key_hpcp = false;
        config.key_hpcp_peaks_per_frame = 0;
        config.key_hpcp_num_harmonics = 0;
        config.key_hpcp_harmonic_decay = 2.0;
        config.key_hpcp_mag_power = -1.0;
        config.enable_key_median = true;
        config.key_median_segment_length_frames = 0;
        config.key_median_segment_hop_frames = 0;
        config.key_median_min_segments = 0;
        config.center_frequency = -1.0;
        config.validate(SAMPLE_RATE, SAMPLE_COUNT).unwrap();

        // Ensemble detection wins over all lower key-decision paths. Their
        // finite-but-otherwise-invalid parameters remain ignored.
        config.enable_key_ensemble = true;
        config.enable_key_multi_scale = true;
        config.key_multi_scale_lengths.clear();
        config.key_multi_scale_hop = 0;
        config.enable_key_segment_voting = true;
        config.key_segment_hop_frames = 0;
        config.enable_key_mode_heuristic = true;
        config.key_mode_third_ratio_margin = -1.0;
        config.key_mode_flip_min_score_ratio = 2.0;
        config.enable_key_minor_harmonic_bonus = true;
        config.key_minor_leading_tone_bonus_weight = -1.0;
        config.debug_gt_bpm = Some(-1.0);
        config.validate(SAMPLE_RATE, SAMPLE_COUNT).unwrap();
    }

    #[test]
    fn analysis_config_respects_key_frontend_precedence() {
        let hpss = AnalysisConfig {
            enable_key_hpss_harmonic: true,
            key_spectrogram_smooth_margin: usize::MAX,
            key_harmonic_mask_power: 0.5,
            ..AnalysisConfig::default()
        };
        hpss.validate(SAMPLE_RATE, SAMPLE_COUNT).unwrap();

        let log_frequency = AnalysisConfig {
            enable_key_log_frequency: true,
            soft_mapping_sigma: -1.0,
            enable_key_tuning_compensation: true,
            key_tuning_frame_step: 0,
            key_tuning_peak_rel_threshold: 2.0,
            key_hpcp_peaks_per_frame: 0,
            key_hpcp_num_harmonics: 0,
            key_hpcp_harmonic_decay: 2.0,
            key_hpcp_mag_power: -1.0,
            enable_key_hpcp_bass_blend: true,
            key_hpcp_bass_fmin_hz: -1.0,
            key_hpcp_bass_fmax_hz: -2.0,
            key_hpcp_bass_weight: 2.0,
            ..AnalysisConfig::default()
        };
        log_frequency.validate(SAMPLE_RATE, SAMPLE_COUNT).unwrap();
    }

    #[test]
    fn analysis_config_rejects_frame_size_one_at_public_boundary() {
        let config = AnalysisConfig {
            frame_size: 1,
            ..AnalysisConfig::default()
        };
        let result = analyze_audio(&[0.5; 64], 44_100, config);
        assert!(matches!(result, Err(AnalysisError::InvalidInput(_))));
    }
}
