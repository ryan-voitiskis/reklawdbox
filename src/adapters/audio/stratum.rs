use std::time::Instant;

use serde::{Deserialize, Serialize};

use super::AudioError;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct StratumResult {
    pub bpm: f64,
    pub bpm_confidence: f64,
    pub key: String,
    pub key_camelot: String,
    pub key_confidence: f64,
    pub key_clarity: f64,
    pub grid_stability: f64,
    /// "rekordbox" when the caller supplied an external grid (e.g. from
    /// ANLZ PQTZ), "hmm" when the analyzer's own beat tracker was used.
    /// Empty string on cached results from before this field existed.
    pub grid_source: String,
    pub duration_seconds: f64,
    pub processing_time_ms: f64,
    pub analyzer_version: String,
    pub mod_centroid: Option<f64>,
    pub harmonic_proportion: Option<f64>,
    pub decay_mid_tau: Option<f64>,
    pub decay_mid_r2: Option<f64>,
    pub decay_high_tau: Option<f64>,
    pub decay_high_r2: Option<f64>,
    /// Number of kick-disjoint stab-band onsets surviving the ±80 ms kick
    /// mask AND (when sections are available) the MainGroove filter.
    /// Populated when the beat grid has at least two beats.
    pub dub_stab_onset_count: Option<u32>,
    /// Stab onsets per second. The denominator depends on
    /// `dub_stab_rate_basis` — MainGroove time when sections detected at
    /// least one MainGroove section, else total track duration. Consumers
    /// comparing rates across tracks should filter to one basis since the
    /// same numeric value means different things under the two
    /// denominators.
    pub dub_stab_onset_rate: Option<f64>,
    /// Either `"main_groove"` or `"track"` — the denominator regime used
    /// for `dub_stab_onset_rate`. `None` when dub_stab itself didn't run.
    pub dub_stab_rate_basis: Option<String>,
    /// 32-bin global beat-relative offset histogram (Stage 2).
    /// Bin 0 is on-beat; bin 16 is the offbeat-eighth. Per-bar histograms
    /// are not surfaced here — callers needing them should run the
    /// analyzer with stratum-dsp directly.
    pub dub_stab_histogram: Option<Vec<f64>>,
    /// Stage 3 — best-matching dub-techno chord-stab template name. One of
    /// `offbeat_eighth`, `all_16th_offbeats`, `anticipation`, `on_beat`,
    /// or `unmatched` when a histogram exists but no template clears
    /// `MIN_TEMPLATE_CONFIDENCE`. `None` here means dub_stab itself didn't
    /// run (no beat grid, Stage 1/2 error) — distinct from "histogram
    /// exists but doesn't fit any template".
    pub dub_stab_template: Option<String>,
    /// Cosine similarity in `[0, 1]` of the histogram against the
    /// best-matching template (regardless of whether that template's name
    /// is surfaced or replaced by `unmatched`).
    pub dub_stab_template_score: Option<f64>,
    /// Kick-pattern detector label. One of `four_on_floor`,
    /// `broken_beat`, `halftime`, `sparse`, or `irregular`.
    pub kick_pattern: Option<String>,
    /// Detector confidence in `[0, 1]`.
    pub kick_pattern_confidence: Option<f64>,
    /// Deduplicated kick-band beat anchors per analysed bar.
    pub kick_kicks_per_bar: Option<f64>,
    /// Number of deduplicated kick-band beat anchors after optional section
    /// filtering.
    pub kick_onset_count: Option<u32>,
    /// Either `"main_groove"` or `"track"` — the section regime used for
    /// kick-pattern aggregation.
    pub kick_rate_basis: Option<String>,
    /// Flattened `4 × 16` kick-placement histogram. Rows are beats within
    /// the bar; columns are sixteenth-subdivision offsets inside each beat.
    pub kick_histogram: Option<Vec<f64>>,
    /// Coarse track structure: ordered list of sections with start/end
    /// times and labels (Intro / MainGroove / Breakdown / Outro). Used by
    /// downstream feature aggregators (kick-pattern, sub-rumble, sidechain
    /// depth) to filter to relevant sections only. `None` when section
    /// detection failed or returned an empty list.
    pub sections: Option<Vec<TrackSectionView>>,
    pub flags: Vec<String>,
    pub warnings: Vec<String>,
}

/// Compact section view surfaced through the cached `StratumResult`.
/// Mirrors `stratum_dsp::features::sections::TrackSection` but with `String`
/// `kind` (instead of an enum) so the JSON cache shape is stable across
/// stratum-dsp version changes that might add new section kinds.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TrackSectionView {
    pub start_seconds: f64,
    pub end_seconds: f64,
    /// One of `"intro"`, `"main_groove"`, `"breakdown"`, `"outro"`.
    pub kind: String,
    pub kick_band_rms: f64,
    pub broadband_rms: f64,
}

pub(crate) fn stratum_notation_to_camelot(stratum_notation: &str) -> String {
    let (num_str, letter) = if stratum_notation.ends_with('A') || stratum_notation.ends_with('B') {
        let (n, l) = stratum_notation.split_at(stratum_notation.len() - 1);
        (n, l)
    } else {
        return stratum_notation.to_string();
    };

    let stratum_num: u32 = match num_str.parse() {
        Ok(n) if (1..=12).contains(&n) => n,
        _ => return stratum_notation.to_string(),
    };

    let camelot_num = (stratum_num + 6) % 12 + 1;
    let camelot_letter = if letter == "A" { "B" } else { "A" };
    format!("{camelot_num}{camelot_letter}")
}

pub fn analyze_with_stratum(
    samples: &[f32],
    sample_rate: u32,
    external_beat_grid: Option<stratum_dsp::BeatGrid>,
) -> Result<StratumResult, AudioError> {
    let grid_source = if external_beat_grid.is_some() {
        "rekordbox"
    } else {
        "hmm"
    };
    let config = stratum_dsp::AnalysisConfig {
        external_beat_grid,
        ..stratum_dsp::AnalysisConfig::default()
    };

    let start = Instant::now();
    let result = stratum_dsp::analyze_audio(samples, sample_rate, config)
        .map_err(|e| AudioError::Analysis(format!("Analysis error: {e}")))?;
    let processing_time_ms = start.elapsed().as_secs_f64() * 1000.0;

    let confidence = stratum_dsp::compute_confidence(&result);

    let duration_seconds = result.metadata.duration_seconds as f64;

    Ok(StratumResult {
        bpm: result.bpm as f64,
        bpm_confidence: confidence.bpm_confidence as f64,
        key: result.key.name(),
        key_camelot: stratum_notation_to_camelot(&result.key.numerical()),
        key_confidence: confidence.key_confidence as f64,
        key_clarity: result.key_clarity as f64,
        grid_stability: confidence.grid_stability as f64,
        grid_source: grid_source.to_string(),
        duration_seconds,
        processing_time_ms,
        analyzer_version: result.metadata.algorithm_version.clone(),
        mod_centroid: result.mod_centroid.map(|v| v as f64),
        harmonic_proportion: result.harmonic_proportion.map(|v| v as f64),
        decay_mid_tau: result
            .decay
            .as_ref()
            .and_then(|d| d.mid.as_ref())
            .map(|b| b.tau_median as f64),
        decay_mid_r2: result
            .decay
            .as_ref()
            .and_then(|d| d.mid.as_ref())
            .map(|b| b.fit_r2_median as f64),
        decay_high_tau: result
            .decay
            .as_ref()
            .and_then(|d| d.high.as_ref())
            .map(|b| b.tau_median as f64),
        decay_high_r2: result
            .decay
            .as_ref()
            .and_then(|d| d.high.as_ref())
            .map(|b| b.fit_r2_median as f64),
        dub_stab_onset_count: result.dub_stab.as_ref().map(|d| d.stab_onset_count),
        dub_stab_onset_rate: result.dub_stab.as_ref().map(|d| d.stab_onset_rate as f64),
        dub_stab_rate_basis: result.dub_stab.as_ref().map(|d| {
            match d.rate_basis {
                stratum_dsp::RateBasis::MainGroove => "main_groove",
                stratum_dsp::RateBasis::Track => "track",
                _ => "unknown",
            }
            .to_string()
        }),
        dub_stab_histogram: result
            .dub_stab
            .as_ref()
            .map(|d| d.histogram.iter().map(|&v| v as f64).collect()),
        // Surface whatever the matcher returned — either a canonical template
        // name (when score >= MIN_TEMPLATE_CONFIDENCE) or the
        // TEMPLATE_UNMATCHED sentinel "unmatched" when a histogram exists but
        // no template clears the threshold. `None` here means dub_stab itself
        // didn't run (no beat grid, Stage 1/2 error), distinct from
        // "histogram exists but no template fits". stratum-dsp does the
        // thresholding so the schema stays stable across threshold tunings.
        dub_stab_template: result
            .dub_stab
            .as_ref()
            .and_then(|d| d.template_match.as_ref())
            .map(|t| t.name.clone()),
        dub_stab_template_score: result
            .dub_stab
            .as_ref()
            .and_then(|d| d.template_match.as_ref())
            .map(|t| t.score as f64),
        kick_pattern: result
            .kick_pattern
            .as_ref()
            .map(|k| k.pattern.as_str().to_string()),
        kick_pattern_confidence: result.kick_pattern.as_ref().map(|k| k.confidence as f64),
        kick_kicks_per_bar: result.kick_pattern.as_ref().map(|k| k.kicks_per_bar as f64),
        kick_onset_count: result.kick_pattern.as_ref().map(|k| k.onset_count),
        kick_rate_basis: result.kick_pattern.as_ref().map(|k| {
            match k.rate_basis {
                stratum_dsp::RateBasis::MainGroove => "main_groove",
                stratum_dsp::RateBasis::Track => "track",
                _ => "unknown",
            }
            .to_string()
        }),
        kick_histogram: result
            .kick_pattern
            .as_ref()
            .map(|k| k.histogram.iter().map(|&v| v as f64).collect()),
        sections: result.sections.as_ref().map(|secs| {
            secs.iter()
                .map(|s| TrackSectionView {
                    start_seconds: s.start_seconds as f64,
                    end_seconds: s.end_seconds as f64,
                    kind: match s.kind {
                        stratum_dsp::features::sections::SectionKind::Intro => "intro",
                        stratum_dsp::features::sections::SectionKind::MainGroove => "main_groove",
                        stratum_dsp::features::sections::SectionKind::Breakdown => "breakdown",
                        stratum_dsp::features::sections::SectionKind::Outro => "outro",
                        // SectionKind is #[non_exhaustive]; future variants
                        // surface as "unknown" until the cache catches up.
                        _ => "unknown",
                    }
                    .to_string(),
                    kick_band_rms: s.kick_band_rms as f64,
                    broadband_rms: s.broadband_rms as f64,
                })
                .collect()
        }),
        flags: result
            .metadata
            .flags
            .iter()
            .map(|f| format!("{f:?}"))
            .collect(),
        warnings: result.metadata.confidence_warnings.clone(),
    })
}
