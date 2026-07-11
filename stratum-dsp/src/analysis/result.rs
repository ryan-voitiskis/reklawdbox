//! Analysis result types

use serde::{Deserialize, Serialize};

use crate::error::AnalysisError;

/// Musical key
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Key {
    /// Major key (0 = C, 1 = C#, ..., 11 = B)
    Major(u32),
    /// Minor key (0 = C, 1 = C#, ..., 11 = B)
    Minor(u32),
}

impl Key {
    /// Get key name in musical notation (e.g., "C", "Am", "F#", "D#m")
    ///
    /// Returns standard musical notation:
    /// - Major keys: note name only (e.g., "C", "C#", "D", "F#")
    /// - Minor keys: note name + "m" (e.g., "Am", "C#m", "Dm", "F#m")
    ///
    /// # Example
    ///
    /// ```
    /// use stratum_dsp::analysis::result::Key;
    ///
    /// assert_eq!(Key::Major(0).name(), "C");
    /// assert_eq!(Key::Major(6).name(), "F#");
    /// assert_eq!(Key::Minor(9).name(), "Am");
    /// assert_eq!(Key::Minor(1).name(), "C#m");
    /// ```
    pub fn name(&self) -> String {
        let note_names = [
            "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
        ];
        match self {
            Key::Major(i) => note_names[*i as usize % 12].to_string(),
            Key::Minor(i) => format!("{}m", note_names[*i as usize % 12]),
        }
    }

    /// Get key in DJ standard numerical notation (e.g., "1A", "2B", "12A")
    ///
    /// Uses the circle of fifths mapping popularized in DJ software:
    /// - Major keys: 1A-12A (1A = C, 2A = G, 3A = D, ..., 12A = F)
    /// - Minor keys: 1B-12B (1B = Am, 2B = Em, 3B = Bm, ..., 12B = Dm)
    ///
    /// The numbering follows the circle of fifths (up a fifth each step).
    /// Keys are displayed in standard musical notation by default (e.g., "C", "Am", "F#").
    ///
    /// # Example
    ///
    /// ```
    /// use stratum_dsp::analysis::result::Key;
    ///
    /// assert_eq!(Key::Major(0).numerical(), "1A");   // C
    /// assert_eq!(Key::Major(7).numerical(), "2A");   // G
    /// assert_eq!(Key::Minor(9).numerical(), "1B");   // Am
    /// assert_eq!(Key::Minor(4).numerical(), "2B");   // Em
    /// ```
    pub fn numerical(&self) -> String {
        // Circle of fifths mapping: C=0, G=7, D=2, A=9, E=4, B=11, F#=6, C#=1, G#=8, D#=3, A#=10, F=5
        // This maps to: 1A, 2A, 3A, 4A, 5A, 6A, 7A, 8A, 9A, 10A, 11A, 12A
        let circle_of_fifths_major = [0, 7, 2, 9, 4, 11, 6, 1, 8, 3, 10, 5]; // C, G, D, A, E, B, F#, C#, G#, D#, A#, F

        match self {
            Key::Major(i) => {
                let key_idx = *i as usize % 12;
                // Find position in circle of fifths
                let position = circle_of_fifths_major
                    .iter()
                    .position(|&x| x == key_idx)
                    .unwrap_or(0);
                format!("{}A", position + 1)
            }
            Key::Minor(i) => {
                let key_idx = *i as usize % 12;
                // Minor keys follow the same circle of fifths pattern
                // Relative minor of 1A (C) is 1B (Am), relative minor of 2A (G) is 2B (Em), etc.
                let circle_of_fifths_minor = [9, 4, 11, 6, 1, 8, 3, 10, 5, 0, 7, 2]; // Am, Em, Bm, F#m, C#m, G#m, D#m, A#m, Fm, Cm, Gm, Dm
                let position = circle_of_fifths_minor
                    .iter()
                    .position(|&x| x == key_idx)
                    .unwrap_or(0);
                format!("{}B", position + 1)
            }
        }
    }

    /// Get key from DJ standard numerical notation
    ///
    /// Converts numerical notation (e.g., "1A", "2B", "12A") back to a Key.
    ///
    /// # Arguments
    ///
    /// * `notation` - Numerical key notation (e.g., "1A", "2B", "12A")
    ///
    /// # Returns
    ///
    /// `Some(Key)` if valid, `None` if invalid format
    ///
    /// # Example
    ///
    /// ```
    /// use stratum_dsp::analysis::result::Key;
    ///
    /// assert_eq!(Key::from_numerical("1A"), Some(Key::Major(0)));   // C
    /// assert_eq!(Key::from_numerical("2A"), Some(Key::Major(7)));   // G
    /// assert_eq!(Key::from_numerical("1B"), Some(Key::Minor(9)));   // Am
    /// assert_eq!(Key::from_numerical("2B"), Some(Key::Minor(4)));   // Em
    /// assert_eq!(Key::from_numerical("0A"), None);  // Invalid
    /// assert_eq!(Key::from_numerical("13A"), None); // Invalid
    /// ```
    pub fn from_numerical(notation: &str) -> Option<Self> {
        if notation.len() < 2 {
            return None;
        }

        let (num_str, suffix) = notation.split_at(notation.len() - 1);
        let num: u32 = num_str.parse().ok()?;

        if !(1..=12).contains(&num) {
            return None;
        }

        // Circle of fifths mapping
        let circle_of_fifths_major = [0, 7, 2, 9, 4, 11, 6, 1, 8, 3, 10, 5]; // C, G, D, A, E, B, F#, C#, G#, D#, A#, F
        let circle_of_fifths_minor = [9, 4, 11, 6, 1, 8, 3, 10, 5, 0, 7, 2]; // Am, Em, Bm, F#m, C#m, G#m, D#m, A#m, Fm, Cm, Gm, Dm

        match suffix {
            "A" => {
                let key_idx = circle_of_fifths_major[(num - 1) as usize];
                Some(Key::Major(key_idx))
            }
            "B" => {
                let key_idx = circle_of_fifths_minor[(num - 1) as usize];
                Some(Key::Minor(key_idx))
            }
            _ => None,
        }
    }
}

/// Beat grid structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeatGrid {
    /// Downbeat times (beat 1) in seconds
    pub downbeats: Vec<f32>,

    /// All beat times in seconds
    pub beats: Vec<f32>,

    /// Bar boundaries in seconds
    pub bars: Vec<f32>,
}

/// Maximum difference between a bar/downbeat and its corresponding beat.
/// Rekordbox PQTZ timestamps are millisecond-quantized.
pub const BEAT_GRID_MATCH_TOLERANCE_SECONDS: f32 = 0.001;

impl BeatGrid {
    /// Validate the public beat-grid contract without sorting or mutating it.
    pub fn validate(&self) -> Result<(), AnalysisError> {
        fn check_series(name: &str, values: &[f32]) -> Result<(), AnalysisError> {
            if let Some((index, value)) = values
                .iter()
                .enumerate()
                .find(|(_, value)| !value.is_finite() || **value < 0.0)
            {
                return Err(AnalysisError::InvalidInput(format!(
                    "beat_grid.{name} must contain finite non-negative values, got {value} at index {index}"
                )));
            }
            if let Some(index) = values.windows(2).position(|pair| pair[0] >= pair[1]) {
                return Err(AnalysisError::InvalidInput(format!(
                    "beat_grid.{name} must be strictly ascending, got {name}[{index}]={} >= {name}[{}]={}",
                    values[index],
                    index + 1,
                    values[index + 1]
                )));
            }
            Ok(())
        }

        check_series("beats", &self.beats)?;
        check_series("downbeats", &self.downbeats)?;
        check_series("bars", &self.bars)?;

        if self.downbeats != self.bars {
            return Err(AnalysisError::InvalidInput(
                "beat_grid.downbeats must equal beat_grid.bars".to_string(),
            ));
        }
        if self.beats.is_empty() {
            if self.bars.is_empty() {
                return Ok(());
            }
            return Err(AnalysisError::InvalidInput(
                "beat_grid.bars cannot be non-empty when beat_grid.beats is empty".to_string(),
            ));
        }

        let first_beat = self.beats[0];
        let last_beat = self.beats[self.beats.len() - 1];
        for (index, &bar) in self.bars.iter().enumerate() {
            if !(first_beat..=last_beat).contains(&bar) {
                return Err(AnalysisError::InvalidInput(format!(
                    "beat_grid.bars[{index}]={bar} must lie within beat range [{first_beat}, {last_beat}]"
                )));
            }
            if !self
                .beats
                .iter()
                .any(|beat| (beat - bar).abs() <= BEAT_GRID_MATCH_TOLERANCE_SECONDS)
            {
                return Err(AnalysisError::InvalidInput(format!(
                    "beat_grid.bars[{index}]={bar} must match a beat within {BEAT_GRID_MATCH_TOLERANCE_SECONDS} seconds"
                )));
            }
        }

        Ok(())
    }
}

/// Analysis flags
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AnalysisFlag {
    /// Multiple BPM peaks equally strong
    MultimodalBpm,
    /// Low key clarity (atonal/ambiguous)
    WeakTonality,
    /// Track has tempo drift
    TempoVariation,
    /// Multiple onset interpretations
    OnsetDetectionAmbiguous,
    /// Beat grid had fewer than 2 beats — dub-stab Stage 1+2 skipped.
    DubStabGridTooShort,
    /// Stage 1 (kick-disjoint stab onset detection) returned an error.
    DubStabStage1Failed,
    /// Stage 2 (beat-relative offset histogram) returned an error.
    DubStabStage2Failed,
    /// Beat grid had fewer than 2 beats — kick-pattern detection skipped.
    KickPatternGridTooShort,
    /// Kick-pattern detection returned an error.
    KickPatternFailed,
    /// Track-section detector returned an error. The downstream pipeline
    /// continues with sections=None and the dub_stab MainGroove filter is
    /// effectively a no-op. Distinct from `NoMainGrooveDetected`.
    SectionDetectionFailed,
    /// Section detection succeeded but identified no MainGroove section
    /// (e.g. continuous-pad ambient track). Dub-stab onset rate is then
    /// denominated by total track duration rather than MainGroove time —
    /// see `DubStabAnalysis.rate_basis`.
    NoMainGrooveDetected,
}

/// Tempogram candidate diagnostics (for validation/tuning)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TempoCandidateDebug {
    /// Candidate tempo in BPM.
    pub bpm: f32,
    /// Combined score used for ranking (blended FFT+autocorr, with mild priors).
    pub score: f32,
    /// FFT-method normalized support in [0, 1].
    pub fft_norm: f32,
    /// Autocorrelation-method normalized support in [0, 1].
    pub autocorr_norm: f32,
    /// True if this candidate was selected as the final BPM.
    pub selected: bool,
}

/// Complete analysis result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisResult {
    /// BPM estimate
    pub bpm: f32,

    /// BPM confidence (0.0-1.0)
    pub bpm_confidence: f32,

    /// Detected key
    pub key: Key,

    /// Key confidence (0.0-1.0)
    pub key_confidence: f32,

    /// Key clarity (0.0-1.0)
    ///
    /// Measures how "tonal" vs "atonal" the track is:
    /// - High (>0.5): Strong tonality, reliable key detection
    /// - Medium (0.2-0.5): Moderate tonality
    /// - Low (<0.2): Weak tonality, key detection may be unreliable
    pub key_clarity: f32,

    /// Beat grid
    pub beat_grid: BeatGrid,

    /// Grid stability (0.0-1.0)
    pub grid_stability: f32,

    /// Analysis metadata
    pub metadata: AnalysisMetadata,

    /// Modulation spectral centroid (Hz).
    /// Measures the average "speed" of amplitude fluctuations across frequency
    /// bands. Low values indicate reverb-smoothed dynamics (dub techno), high
    /// values indicate crisp transients (techno).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mod_centroid: Option<f32>,

    /// Harmonic proportion H/(H+P) from HPSS decomposition.
    /// Bounded [0, 1]: near 1.0 = pad-dominated (dub techno),
    /// near 0.0 = percussion-dominated (techno).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub harmonic_proportion: Option<f32>,

    /// Post-transient spectral decay rates in two frequency bands.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decay: Option<crate::features::decay::DecayResult>,

    /// Dub-stab Stage 1 + Stage 2: kick-disjoint stab-band onset count and
    /// the beat-relative offset histogram. Populated when a beat grid is
    /// available (either from the HMM tracker or supplied externally).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dub_stab: Option<DubStabAnalysis>,

    /// Kick-pattern detector result: beat-relative low-band onset placement.
    /// Populated when a beat grid is available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kick_pattern: Option<KickPatternAnalysis>,

    /// Coarse track-level section labels (Intro / MainGroove / Breakdown /
    /// Outro). Used by downstream feature aggregators to filter to
    /// kick-active or main-groove sections.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sections: Option<Vec<crate::features::sections::TrackSection>>,
}

/// Coarse kick placement category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum KickPattern {
    /// Strong kick on every quarter-note beat.
    FourOnFloor,
    /// Kick placement is syncopated or breakbeat-like.
    BrokenBeat,
    /// Kick placement emphasizes beats 1 and 3 at a faster notated tempo.
    Halftime,
    /// Too few kick-band onsets to establish a recurring pattern.
    Sparse,
    /// Kick-band onsets exist but do not fit a supported template.
    Irregular,
}

impl KickPattern {
    /// Stable lower-snake-case label for cache and MCP consumers.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FourOnFloor => "four_on_floor",
            Self::BrokenBeat => "broken_beat",
            Self::Halftime => "halftime",
            Self::Sparse => "sparse",
            Self::Irregular => "irregular",
        }
    }
}

/// Kick-pattern detector output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KickPatternAnalysis {
    /// Detected coarse kick placement category.
    pub pattern: KickPattern,
    /// Detector confidence in `[0, 1]`.
    pub confidence: f32,
    /// Detected kick-band beat anchors per analysed bar.
    pub kicks_per_bar: f32,
    /// Number of kick-band beat anchors used after optional section filtering.
    pub onset_count: u32,
    /// Flattened `4 × 16` beat-relative histogram. Rows are beats within
    /// the bar; columns are sixteenth-subdivision offsets inside each beat.
    pub histogram: Vec<f32>,
    /// Whether section filtering limited the detector to MainGroove or used
    /// the whole track.
    pub rate_basis: RateBasis,
}

/// Dub-stab Stage 1 + Stage 2 + Stage 3 result, surfaced from the
/// top-level pipeline.
///
/// `histogram` is always 32-element; `per_bar_histograms` has one 32-element
/// inner vec per bar in the beat grid. `template_match`, when present,
/// names the canonical chord-stab pattern that best fits the global
/// histogram (Stage 3) — see `features::dub_stab::dub_stab_templates` for
/// the bank.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DubStabAnalysis {
    /// Kick-disjoint stab-band onsets surviving Stage 1's ±80 ms kick mask
    /// AND (when sections are available) the MainGroove filter. `u32` (not
    /// `usize`) so the serialised JSON is platform-independent.
    pub stab_onset_count: u32,
    /// Stab onsets per second. The denominator depends on `rate_basis` —
    /// MainGroove time when sections detected at least one MainGroove
    /// section, else total track duration. Consumers comparing rates
    /// across tracks should check `rate_basis` first or filter to one
    /// regime, since the same numeric value means different things under
    /// the two denominators.
    pub stab_onset_rate: f32,
    /// Whether `stab_onset_rate` was computed against MainGroove duration
    /// or against total track duration. See `RateBasis`.
    pub rate_basis: RateBasis,
    /// Global histogram aggregated across the analysed window. Length 32.
    pub histogram: Vec<f32>,
    /// One sub-histogram per bar in the beat grid. Each is length 32.
    /// Bars outside MainGroove sections (when sections are available) will
    /// have empty histograms because no onsets are attributed to them.
    pub per_bar_histograms: Vec<Vec<f32>>,
    /// Stage 3 — best-matching template name + cosine similarity.
    /// `None` when the histogram is all zero (no signal to match).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template_match: Option<DubStabTemplateMatch>,
}

/// Denominator regime for `DubStabAnalysis.stab_onset_rate`. Surfaced so
/// consumers comparing rates across tracks can filter to a single regime
/// rather than silently mixing apples and oranges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RateBasis {
    /// Rate denominator was the sum of MainGroove section durations.
    MainGroove,
    /// Rate denominator was the entire track duration. Either no
    /// sections were detected, or sections existed but contained no
    /// MainGroove entry.
    Track,
}

/// Compact serialisable view of `features::dub_stab::TemplateMatch` for
/// inclusion in `AnalysisResult`. Drops `all_scores` to keep the cached
/// JSON small; callers wanting the full vector should call
/// `features::dub_stab::match_template` directly on the histogram.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DubStabTemplateMatch {
    /// Best-matching template name (e.g. `"offbeat_eighth"`).
    pub name: String,
    /// Cosine similarity in `[0, 1]` between the histogram and the named
    /// template. Higher = better match.
    pub score: f32,
}

/// Analysis metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisMetadata {
    /// Audio duration in seconds
    pub duration_seconds: f32,

    /// Sample rate in Hz
    pub sample_rate: u32,

    /// Processing time in milliseconds
    pub processing_time_ms: f32,

    /// Algorithm version
    pub algorithm_version: String,

    /// Onset method consensus score
    pub onset_method_consensus: f32,

    /// Methods used
    pub methods_used: Vec<String>,

    /// Analysis flags
    pub flags: Vec<AnalysisFlag>,

    /// Confidence warnings (low confidence, ambiguous results, etc.)
    pub confidence_warnings: Vec<String>,

    /// Optional: tempogram candidate list (top-N) for diagnostics.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tempogram_candidates: Option<Vec<TempoCandidateDebug>>,

    /// Tempogram multi-resolution escalation was triggered for this track (ambiguous base estimate).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tempogram_multi_res_triggered: Option<bool>,

    /// Tempogram multi-resolution result was selected over the base single-resolution estimate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tempogram_multi_res_used: Option<bool>,

    /// Tempogram percussive-only fallback was triggered (ambiguous estimate + HPSS enabled).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tempogram_percussive_triggered: Option<bool>,

    /// Tempogram percussive-only fallback was selected over the current estimate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tempogram_percussive_used: Option<bool>,
}

// Re-export for convenience
pub use Key as KeyType;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_beat_grid_validation_contract_and_tolerance() {
        let valid = BeatGrid {
            beats: vec![0.0, 0.5, 1.0],
            downbeats: vec![0.000_999],
            bars: vec![0.000_999],
        };
        assert!(valid.validate().is_ok());

        let outside_tolerance = BeatGrid {
            downbeats: vec![0.001_001],
            bars: vec![0.001_001],
            ..valid.clone()
        };
        assert!(matches!(
            outside_tolerance.validate(),
            Err(AnalysisError::InvalidInput(_))
        ));

        for invalid in [
            BeatGrid {
                beats: vec![0.0, f32::NAN],
                downbeats: vec![],
                bars: vec![],
            },
            BeatGrid {
                beats: vec![0.0, 0.5, 0.5],
                downbeats: vec![],
                bars: vec![],
            },
            BeatGrid {
                beats: vec![0.0, 0.75, 0.5],
                downbeats: vec![],
                bars: vec![],
            },
            BeatGrid {
                beats: vec![],
                downbeats: vec![0.0],
                bars: vec![0.0],
            },
            BeatGrid {
                beats: vec![0.0, 0.5],
                downbeats: vec![0.0],
                bars: vec![],
            },
            BeatGrid {
                beats: vec![0.0, 0.5],
                downbeats: vec![1.0],
                bars: vec![1.0],
            },
        ] {
            assert!(matches!(
                invalid.validate(),
                Err(AnalysisError::InvalidInput(_))
            ));
        }

        assert!(BeatGrid {
            beats: vec![],
            downbeats: vec![],
            bars: vec![],
        }
        .validate()
        .is_ok());
    }

    #[test]
    fn test_key_name_major() {
        assert_eq!(Key::Major(0).name(), "C");
        assert_eq!(Key::Major(1).name(), "C#");
        assert_eq!(Key::Major(2).name(), "D");
        assert_eq!(Key::Major(6).name(), "F#");
        assert_eq!(Key::Major(11).name(), "B");
    }

    #[test]
    fn test_key_name_minor() {
        assert_eq!(Key::Minor(0).name(), "Cm");
        assert_eq!(Key::Minor(1).name(), "C#m");
        assert_eq!(Key::Minor(2).name(), "Dm");
        assert_eq!(Key::Minor(9).name(), "Am");
        assert_eq!(Key::Minor(11).name(), "Bm");
    }

    #[test]
    fn test_key_numerical_major() {
        // Circle of fifths: C=1A, G=2A, D=3A, A=4A, E=5A, B=6A, F#=7A, C#=8A, G#=9A, D#=10A, A#=11A, F=12A
        assert_eq!(Key::Major(0).numerical(), "1A"); // C
        assert_eq!(Key::Major(7).numerical(), "2A"); // G
        assert_eq!(Key::Major(2).numerical(), "3A"); // D
        assert_eq!(Key::Major(9).numerical(), "4A"); // A
        assert_eq!(Key::Major(4).numerical(), "5A"); // E
        assert_eq!(Key::Major(11).numerical(), "6A"); // B
        assert_eq!(Key::Major(6).numerical(), "7A"); // F#
        assert_eq!(Key::Major(1).numerical(), "8A"); // C#
        assert_eq!(Key::Major(8).numerical(), "9A"); // G#
        assert_eq!(Key::Major(3).numerical(), "10A"); // D#
        assert_eq!(Key::Major(10).numerical(), "11A"); // A#
        assert_eq!(Key::Major(5).numerical(), "12A"); // F
    }

    #[test]
    fn test_key_numerical_minor() {
        // Circle of fifths: Am=1B, Em=2B, Bm=3B, F#m=4B, C#m=5B, G#m=6B, D#m=7B, A#m=8B, Fm=9B, Cm=10B, Gm=11B, Dm=12B
        assert_eq!(Key::Minor(9).numerical(), "1B"); // Am
        assert_eq!(Key::Minor(4).numerical(), "2B"); // Em
        assert_eq!(Key::Minor(11).numerical(), "3B"); // Bm
        assert_eq!(Key::Minor(6).numerical(), "4B"); // F#m
        assert_eq!(Key::Minor(1).numerical(), "5B"); // C#m
        assert_eq!(Key::Minor(8).numerical(), "6B"); // G#m
        assert_eq!(Key::Minor(3).numerical(), "7B"); // D#m
        assert_eq!(Key::Minor(10).numerical(), "8B"); // A#m
        assert_eq!(Key::Minor(5).numerical(), "9B"); // Fm
        assert_eq!(Key::Minor(0).numerical(), "10B"); // Cm
        assert_eq!(Key::Minor(7).numerical(), "11B"); // Gm
        assert_eq!(Key::Minor(2).numerical(), "12B"); // Dm
    }

    #[test]
    fn test_key_from_numerical() {
        // Test major keys
        assert_eq!(Key::from_numerical("1A"), Some(Key::Major(0))); // C
        assert_eq!(Key::from_numerical("2A"), Some(Key::Major(7))); // G
        assert_eq!(Key::from_numerical("7A"), Some(Key::Major(6))); // F#
        assert_eq!(Key::from_numerical("12A"), Some(Key::Major(5))); // F

        // Test minor keys
        assert_eq!(Key::from_numerical("1B"), Some(Key::Minor(9))); // Am
        assert_eq!(Key::from_numerical("2B"), Some(Key::Minor(4))); // Em
        assert_eq!(Key::from_numerical("10B"), Some(Key::Minor(0))); // Cm

        // Test invalid inputs
        assert_eq!(Key::from_numerical("0A"), None);
        assert_eq!(Key::from_numerical("13A"), None);
        assert_eq!(Key::from_numerical("1C"), None);
        assert_eq!(Key::from_numerical(""), None);
        assert_eq!(Key::from_numerical("A"), None);
    }

    #[test]
    fn test_key_numerical_roundtrip() {
        // Test that numerical() and from_numerical() are inverses
        for i in 0..12 {
            let major = Key::Major(i);
            let num = major.numerical();
            assert_eq!(
                Key::from_numerical(&num),
                Some(major),
                "Failed roundtrip for major key {}: {}",
                i,
                num
            );

            let minor = Key::Minor(i);
            let num = minor.numerical();
            assert_eq!(
                Key::from_numerical(&num),
                Some(minor),
                "Failed roundtrip for minor key {}: {}",
                i,
                num
            );
        }
    }
}
