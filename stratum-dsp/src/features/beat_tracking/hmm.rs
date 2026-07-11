//! HMM Viterbi beat tracker
//!
//! Uses Hidden Markov Model with Viterbi algorithm to track beat sequence.
//!
//! This module implements beat tracking using the HMM Viterbi algorithm as described
//! in Böck et al. (2016). The algorithm models tempo variations as hidden states and
//! finds the globally optimal beat sequence by maximizing the probability of observed
//! onsets given the tempo model.
//!
//! # Algorithm
//!
//! 1. **State Space Construction**: Create 5 states representing BPM variations
//!    around the nominal estimate (±10% in 5% steps)
//! 2. **Transition Probabilities**: Model tempo stability (staying at same tempo is
//!    most likely, small changes are possible, large changes are unlikely)
//! 3. **Emission Probabilities**: Model onset alignment with expected beats using
//!    Gaussian decay based on distance to nearest onset
//! 4. **Viterbi Forward Pass**: Compute best path probability for each state at each time
//! 5. **Backtracking**: Extract most likely beat sequence from the computed path
//!
//! # Reference
//!
//! Böck, S., Krebs, F., & Schedl, M. (2016). Joint Beat and Downbeat Tracking with a
//! Recurrent Neural Network. *Proceedings of the International Society for Music
//! Information Retrieval Conference*.
//!
//! # Example
//!
//! ```no_run
//! use stratum_dsp::features::beat_tracking::hmm::HmmBeatTracker;
//!
//! let bpm_estimate = 120.0;
//! let onsets = vec![0.0, 0.5, 1.0, 1.5]; // Onset times in seconds
//! let sample_rate = 44100;
//!
//! let tracker = HmmBeatTracker::new(bpm_estimate, onsets, sample_rate);
//! let beats = tracker.track_beats()?;
//!
//! for beat in beats {
//!     println!("Beat at {:.3}s: confidence={:.2}", beat.time_seconds, beat.confidence);
//! }
//! # Ok::<(), stratum_dsp::AnalysisError>(())
//! ```

use super::BeatPosition;
use crate::error::AnalysisError;

/// Numerical stability epsilon
const EPSILON: f32 = 1e-10;

/// Number of states in HMM (BPM variations: -10%, -5%, 0%, +5%, +10%)
const NUM_STATES: usize = 5;

/// Timing tolerance for emission probability (50ms in seconds)
const TIMING_TOLERANCE_S: f32 = 0.05;

/// Standard deviation for Gaussian emission probability (σ = tolerance / 2)
const EMISSION_SIGMA: f32 = TIMING_TOLERANCE_S / 2.0;

/// Floor for emission probability (exp(-10) ≈ 4.5e-5). Applied to out-of-range
/// frames so they get a low but non-zero value in log-space Viterbi.
const EMISSION_FLOOR: f32 = 4.539_993e-5; // (-10.0_f32).exp()

/// Minimum emission probability to consider a frame as a beat during extraction.
/// Must be strictly greater than EMISSION_FLOOR.
const EMISSION_THRESHOLD: f32 = 0.1;

#[derive(Debug, Clone, Copy)]
struct HmmCell {
    log_probability: f32,
    predecessor_state: Option<usize>,
    expected_time: f32,
    emission_probability: f32,
    onset_distance: f32,
}

#[derive(Debug, Clone, Copy)]
struct PathStep {
    state: usize,
    expected_time: f32,
    emission_probability: f32,
    onset_distance: f32,
}

#[derive(Debug)]
struct ViterbiTrace {
    steps: Vec<PathStep>,
    terminal_row: usize,
    terminal_state: usize,
    max_steps: usize,
}

/// HMM beat tracker
#[derive(Debug)]
pub struct HmmBeatTracker {
    /// BPM estimate
    pub bpm_estimate: f32,

    /// Onset times in seconds
    pub onsets: Vec<f32>,

    /// Sample rate in Hz
    pub sample_rate: u32,
}

impl HmmBeatTracker {
    /// Create a new HMM beat tracker
    ///
    /// # Arguments
    ///
    /// * `bpm_estimate` - Nominal BPM estimate from period estimation
    /// * `onsets` - Onset times in seconds (must be sorted)
    /// * `sample_rate` - Sample rate in Hz (for logging/debugging)
    ///
    /// # Panics
    ///
    /// Does not panic, but will return errors if inputs are invalid
    pub fn new(bpm_estimate: f32, onsets: Vec<f32>, sample_rate: u32) -> Self {
        Self {
            bpm_estimate,
            onsets,
            sample_rate,
        }
    }

    /// Track beats using Viterbi algorithm
    ///
    /// Finds the globally optimal beat sequence by modeling tempo variations as
    /// hidden states and maximizing the probability of observed onsets.
    ///
    /// # Returns
    ///
    /// Vector of beat positions with confidence scores, sorted by time
    ///
    /// # Errors
    ///
    /// Returns `AnalysisError` if:
    /// - BPM estimate is invalid (≤ 0 or > 300)
    /// - Onsets list is empty
    /// - Numerical errors occur during computation
    ///
    /// # Algorithm Details
    ///
    /// The algorithm uses a 5-state HMM where each state represents a BPM variation:
    /// - State 0: -10% (slower)
    /// - State 1: -5% (slightly slower)
    /// - State 2: 0% (nominal)
    /// - State 3: +5% (slightly faster)
    /// - State 4: +10% (faster)
    ///
    /// Transition probabilities favor tempo stability (staying at same state or
    /// transitioning to adjacent states). Emission probabilities use Gaussian decay
    /// based on distance to nearest onset.
    pub fn track_beats(&self) -> Result<Vec<BeatPosition>, AnalysisError> {
        self.validate_inputs()?;

        log::debug!(
            "Tracking beats with HMM: BPM={:.2}, {} onsets",
            self.bpm_estimate,
            self.onsets.len()
        );

        let state_bpms = self.build_state_space();
        let transition_matrix = self.build_transition_matrix();
        let trace = self.viterbi_trace(&state_bpms, &transition_matrix)?;
        log::trace!(
            "HMM terminal: row={}, state={}, horizon={}",
            trace.terminal_row,
            trace.terminal_state,
            trace.max_steps
        );
        if trace
            .steps
            .iter()
            .any(|step| step.state >= state_bpms.len())
        {
            return Err(AnalysisError::ProcessingError(
                "HMM trace contains an invalid tempo state".to_string(),
            ));
        }
        let beats = self.extract_beats_from_trace(&trace)?;

        log::debug!("HMM beat tracking complete: {} beats detected", beats.len());

        Ok(beats)
    }

    fn validate_inputs(&self) -> Result<(), AnalysisError> {
        if !self.bpm_estimate.is_finite()
            || self.bpm_estimate <= EPSILON
            || self.bpm_estimate > 300.0
        {
            return Err(AnalysisError::InvalidInput(format!(
                "Invalid BPM estimate: {:.2} (must be > 0 and <= 300)",
                self.bpm_estimate
            )));
        }

        if self.onsets.is_empty() {
            return Err(AnalysisError::InvalidInput(
                "Cannot track beats: no onsets provided".to_string(),
            ));
        }

        for (index, onset) in self.onsets.iter().copied().enumerate() {
            if !onset.is_finite() || onset < 0.0 {
                return Err(AnalysisError::InvalidInput(format!(
                    "Invalid onset at index {index}: {onset} (must be finite and non-negative)"
                )));
            }
        }
        if let Some((index, pair)) = self
            .onsets
            .windows(2)
            .enumerate()
            .find(|(_, pair)| pair[0] > pair[1])
        {
            return Err(AnalysisError::InvalidInput(format!(
                "Onsets must be sorted: onsets[{index}]={} exceeds onsets[{}]={}",
                pair[0],
                index + 1,
                pair[1]
            )));
        }

        Ok(())
    }

    /// Build state space: 5 states representing BPM variations
    ///
    /// Returns vector of BPM values: [0.9*bpm, 0.95*bpm, bpm, 1.05*bpm, 1.1*bpm]
    fn build_state_space(&self) -> Vec<f32> {
        let mut states = Vec::with_capacity(NUM_STATES);
        let multipliers = [0.90, 0.95, 1.00, 1.05, 1.10];

        for &mult in &multipliers {
            states.push(self.bpm_estimate * mult);
        }

        states
    }

    /// Build transition probability matrix
    ///
    /// Models tempo stability:
    /// - Self-transition: 0.7 (most likely to stay at same tempo)
    /// - Adjacent states: 0.15 each (small tempo changes are possible)
    /// - Distant states: 0.0 (large tempo changes are unlikely)
    ///
    /// Returns NUM_STATES × NUM_STATES matrix
    fn build_transition_matrix(&self) -> Vec<Vec<f32>> {
        let mut matrix = vec![vec![0.0; NUM_STATES]; NUM_STATES];

        #[allow(clippy::needless_range_loop)]
        for i in 0..NUM_STATES {
            for j in 0..NUM_STATES {
                let distance = (i as i32 - j as i32).unsigned_abs() as usize;

                if distance == 0 {
                    // Self-transition: most likely
                    matrix[i][j] = 0.7;
                } else if distance == 1 {
                    // Adjacent state: possible
                    matrix[i][j] = 0.15;
                } else {
                    // Distant state: unlikely
                    matrix[i][j] = 0.0;
                }
            }
        }

        // Normalize rows (should sum to 1.0)
        for row in &mut matrix {
            let sum: f32 = row.iter().sum();
            if sum > EPSILON {
                for val in row.iter_mut() {
                    *val /= sum;
                }
            }
        }

        matrix
    }

    fn nearest_onset_emission(&self, expected_time: f32) -> Result<(f32, f32), AnalysisError> {
        if !expected_time.is_finite() || expected_time < 0.0 {
            return Err(AnalysisError::NumericalError(format!(
                "Invalid expected beat time: {expected_time}"
            )));
        }
        let sigma_sq = EMISSION_SIGMA * EMISSION_SIGMA;
        if sigma_sq <= EPSILON {
            return Err(AnalysisError::NumericalError(
                "Emission sigma too small".to_string(),
            ));
        }

        let min_distance = self
            .onsets
            .iter()
            .map(|onset| (*onset - expected_time).abs())
            .min_by(f32::total_cmp)
            .ok_or_else(|| AnalysisError::InvalidInput("No onsets available".to_string()))?;
        let emission = (-(min_distance * min_distance) / (2.0 * sigma_sq))
            .exp()
            .max(EMISSION_FLOOR);

        if !emission.is_finite() || !min_distance.is_finite() {
            return Err(AnalysisError::NumericalError(
                "Non-finite HMM emission".to_string(),
            ));
        }

        Ok((emission, min_distance))
    }

    fn checked_timing_bounds(
        &self,
        state_bpms: &[f32],
    ) -> Result<(f32, f32, usize), AnalysisError> {
        let min_bpm = state_bpms.iter().copied().min_by(f32::total_cmp);
        let max_bpm = state_bpms.iter().copied().max_by(f32::total_cmp);
        let (Some(min_bpm), Some(max_bpm)) = (min_bpm, max_bpm) else {
            return Err(AnalysisError::ProcessingError(
                "Cannot derive HMM horizon without tempo states".to_string(),
            ));
        };
        if !min_bpm.is_finite() || !max_bpm.is_finite() || min_bpm <= 0.0 {
            return Err(AnalysisError::NumericalError(
                "Invalid HMM tempo state range".to_string(),
            ));
        }

        let min_interval = 60.0 / max_bpm;
        let max_interval = 60.0 / min_bpm;
        let duration = self.onsets[self.onsets.len() - 1] as f64 - self.onsets[0] as f64;
        let raw_steps = (duration + max_interval as f64) / min_interval as f64;
        if !min_interval.is_finite()
            || !max_interval.is_finite()
            || min_interval <= 0.0
            || max_interval <= 0.0
            || !duration.is_finite()
            || duration < 0.0
            || !raw_steps.is_finite()
            || raw_steps < 0.0
        {
            return Err(AnalysisError::NumericalError(
                "Cannot derive a finite HMM iteration horizon".to_string(),
            ));
        }

        let rounded_steps = raw_steps.ceil();
        if rounded_steps > (usize::MAX - 1) as f64 {
            return Err(AnalysisError::NumericalError(
                "HMM iteration horizon exceeds platform capacity".to_string(),
            ));
        }
        let max_steps = (rounded_steps as usize).checked_add(1).ok_or_else(|| {
            AnalysisError::NumericalError("HMM iteration horizon overflow".to_string())
        })?;

        Ok((min_interval, max_interval, max_steps))
    }

    fn next_expected_time(predecessor_time: f32, destination_bpm: f32) -> Option<f32> {
        if !predecessor_time.is_finite() || !destination_bpm.is_finite() || destination_bpm <= 0.0 {
            return None;
        }
        let candidate_time = predecessor_time + 60.0 / destination_bpm;
        (candidate_time.is_finite() && candidate_time > predecessor_time).then_some(candidate_time)
    }

    fn viterbi_trace(
        &self,
        state_bpms: &[f32],
        transition_matrix: &[Vec<f32>],
    ) -> Result<ViterbiTrace, AnalysisError> {
        let (min_interval, max_interval, max_steps) = self.checked_timing_bounds(state_bpms)?;
        let end_time = self.onsets[self.onsets.len() - 1];
        let terminal_start = end_time - min_interval;
        let extension_limit = end_time + max_interval;

        let log_transition: Vec<Vec<f32>> = transition_matrix
            .iter()
            .map(|row| {
                row.iter()
                    .map(|&p| {
                        if p > EPSILON {
                            p.ln()
                        } else {
                            f32::NEG_INFINITY
                        }
                    })
                    .collect()
            })
            .collect();

        let mut rows = Vec::new();
        rows.try_reserve(max_steps).map_err(|error| {
            AnalysisError::ProcessingError(format!(
                "Cannot allocate HMM iteration horizon ({max_steps} rows): {error}"
            ))
        })?;

        let start_time = self.onsets[0];
        let (initial_emission, initial_distance) = self.nearest_onset_emission(start_time)?;
        let log_initial = (1.0 / NUM_STATES as f32).ln();
        let initial_path_log = log_initial + initial_emission.max(EPSILON).ln();
        let mut initial_row = vec![None; NUM_STATES];
        initial_row.fill(Some(HmmCell {
            log_probability: initial_path_log,
            predecessor_state: None,
            expected_time: start_time,
            emission_probability: initial_emission,
            onset_distance: initial_distance,
        }));
        rows.push(initial_row);

        for row_index in 1..max_steps {
            let previous_row = rows.last().ok_or_else(|| {
                AnalysisError::ProcessingError("HMM trace lost its initial row".to_string())
            })?;
            let mut next_row = vec![None; NUM_STATES];

            for (destination_state, destination_bpm) in state_bpms.iter().copied().enumerate() {
                let mut best_cell: Option<HmmCell> = None;
                for (predecessor_state, predecessor_cell) in
                    previous_row.iter().copied().enumerate()
                {
                    let Some(predecessor_cell) = predecessor_cell else {
                        continue;
                    };
                    let transition_log = log_transition
                        .get(predecessor_state)
                        .and_then(|row| row.get(destination_state))
                        .copied()
                        .unwrap_or(f32::NEG_INFINITY);
                    if !transition_log.is_finite() {
                        continue;
                    }
                    let Some(candidate_time) =
                        Self::next_expected_time(predecessor_cell.expected_time, destination_bpm)
                    else {
                        continue;
                    };
                    if candidate_time > extension_limit {
                        continue;
                    }
                    let (emission_probability, onset_distance) =
                        self.nearest_onset_emission(candidate_time)?;
                    let log_probability = predecessor_cell.log_probability
                        + transition_log
                        + emission_probability.max(EPSILON).ln();
                    if !log_probability.is_finite() {
                        continue;
                    }
                    let candidate = HmmCell {
                        log_probability,
                        predecessor_state: Some(predecessor_state),
                        expected_time: candidate_time,
                        emission_probability,
                        onset_distance,
                    };
                    let replace = match best_cell {
                        None => true,
                        Some(current) => {
                            log_probability.total_cmp(&current.log_probability).is_gt()
                                || (log_probability.total_cmp(&current.log_probability).is_eq()
                                    && (candidate_time.total_cmp(&current.expected_time).is_lt()
                                        || (candidate_time
                                            .total_cmp(&current.expected_time)
                                            .is_eq()
                                            && predecessor_state
                                                < current.predecessor_state.unwrap_or(usize::MAX))))
                        }
                    };
                    if replace {
                        best_cell = Some(candidate);
                    }
                }
                next_row[destination_state] = best_cell;
            }

            if next_row.iter().all(Option::is_none) {
                break;
            }
            rows.push(next_row);

            if row_index + 1 == max_steps {
                break;
            }
        }

        let mut best_terminal: Option<(usize, usize, HmmCell)> = None;
        for (row_index, row) in rows.iter().enumerate() {
            for (state, cell) in row.iter().copied().enumerate() {
                let Some(cell) = cell else {
                    continue;
                };
                if cell.expected_time < terminal_start || cell.expected_time > extension_limit {
                    continue;
                }
                let mean_log_score = if row_index == 0 {
                    cell.log_probability
                } else {
                    (cell.log_probability - initial_path_log) / row_index as f32
                };
                let replace = match best_terminal {
                    None => true,
                    Some((best_row, best_state, best_cell)) => {
                        let best_mean = if best_row == 0 {
                            best_cell.log_probability
                        } else {
                            (best_cell.log_probability - initial_path_log) / best_row as f32
                        };
                        mean_log_score.total_cmp(&best_mean).is_gt()
                            || (mean_log_score.total_cmp(&best_mean).is_eq()
                                && (cell
                                    .log_probability
                                    .total_cmp(&best_cell.log_probability)
                                    .is_gt()
                                    || (cell
                                        .log_probability
                                        .total_cmp(&best_cell.log_probability)
                                        .is_eq()
                                        && (state < best_state
                                            || (state == best_state && row_index < best_row)))))
                    }
                };
                if replace {
                    best_terminal = Some((row_index, state, cell));
                }
            }
        }
        let (terminal_row, terminal_state, _) = best_terminal.ok_or_else(|| {
            AnalysisError::ProcessingError("No HMM path reached the final-onset region".to_string())
        })?;

        let mut steps = Vec::with_capacity(terminal_row + 1);
        let mut row_index = terminal_row;
        let mut state = terminal_state;
        loop {
            let cell = rows[row_index][state].ok_or_else(|| {
                AnalysisError::ProcessingError(format!(
                    "Missing HMM cell while backtracking row {row_index}, state {state}"
                ))
            })?;
            steps.push(PathStep {
                state,
                expected_time: cell.expected_time,
                emission_probability: cell.emission_probability,
                onset_distance: cell.onset_distance,
            });
            if row_index == 0 {
                break;
            }
            state = cell.predecessor_state.ok_or_else(|| {
                AnalysisError::ProcessingError(format!(
                    "Missing HMM predecessor at row {row_index}, state {state}"
                ))
            })?;
            row_index -= 1;
        }
        steps.reverse();

        if steps.windows(2).any(|pair| {
            !pair[0].expected_time.is_finite()
                || !pair[1].expected_time.is_finite()
                || pair[0].expected_time >= pair[1].expected_time
        }) {
            return Err(AnalysisError::ProcessingError(
                "Backtracked HMM path is not cumulatively increasing".to_string(),
            ));
        }

        Ok(ViterbiTrace {
            steps,
            terminal_row,
            terminal_state,
            max_steps,
        })
    }

    fn extract_beats_from_trace(
        &self,
        trace: &ViterbiTrace,
    ) -> Result<Vec<BeatPosition>, AnalysisError> {
        let mut beats = Vec::new();
        for step in &trace.steps {
            if step.emission_probability <= EMISSION_THRESHOLD {
                continue;
            }
            if !step.expected_time.is_finite() || step.expected_time < 0.0 {
                return Err(AnalysisError::ProcessingError(
                    "HMM path contains an invalid beat time".to_string(),
                ));
            }
            let alignment_score = if step.onset_distance < TIMING_TOLERANCE_S {
                1.0 - step.onset_distance / TIMING_TOLERANCE_S
            } else {
                0.0
            };
            let confidence = (step.emission_probability * 0.7 + alignment_score * 0.3).min(1.0);
            if !confidence.is_finite() || !(0.0..=1.0).contains(&confidence) {
                return Err(AnalysisError::NumericalError(
                    "HMM path contains an invalid confidence".to_string(),
                ));
            }
            beats.push(BeatPosition {
                beat_index: 0,
                time_seconds: step.expected_time,
                confidence,
            });
        }

        if beats.windows(2).any(|pair| {
            !pair[0].time_seconds.is_finite()
                || !pair[1].time_seconds.is_finite()
                || pair[0].time_seconds >= pair[1].time_seconds
        }) {
            return Err(AnalysisError::ProcessingError(
                "Extracted HMM path is not cumulatively increasing".to_string(),
            ));
        }

        beats.sort_by(|left, right| left.time_seconds.total_cmp(&right.time_seconds));
        let merge_tolerance = TIMING_TOLERANCE_S.min(0.20 * (60.0 / self.bpm_estimate));
        let mut merged: Vec<BeatPosition> = Vec::with_capacity(beats.len());
        for beat in beats {
            if let Some(previous) = merged.last_mut() {
                if beat.time_seconds - previous.time_seconds < merge_tolerance {
                    if beat.confidence > previous.confidence {
                        *previous = beat;
                    }
                    continue;
                }
            }
            merged.push(beat);
        }

        if merged.windows(2).any(|pair| {
            !pair[0].time_seconds.is_finite()
                || !pair[1].time_seconds.is_finite()
                || pair[0].time_seconds >= pair[1].time_seconds
        }) {
            return Err(AnalysisError::ProcessingError(
                "Merged HMM beat path is not strictly increasing".to_string(),
            ));
        }

        Ok(merged)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_emission_floor_below_threshold() {
        const _: () = assert!(
            EMISSION_FLOOR < EMISSION_THRESHOLD,
            "EMISSION_FLOOR must be < EMISSION_THRESHOLD"
        );
    }

    #[test]
    fn test_hmm_tracker_creation() {
        let tracker = HmmBeatTracker::new(120.0, vec![0.0, 0.5, 1.0], 44100);
        assert_eq!(tracker.bpm_estimate, 120.0);
        assert_eq!(tracker.onsets.len(), 3);
        assert_eq!(tracker.sample_rate, 44100);
    }

    #[test]
    fn test_build_state_space() {
        let tracker = HmmBeatTracker::new(120.0, vec![0.0], 44100);
        let states = tracker.build_state_space();

        assert_eq!(states.len(), 5);
        assert!((states[0] - 108.0).abs() < 0.1); // 0.9 * 120
        assert!((states[1] - 114.0).abs() < 0.1); // 0.95 * 120
        assert!((states[2] - 120.0).abs() < 0.1); // 1.0 * 120
        assert!((states[3] - 126.0).abs() < 0.1); // 1.05 * 120
        assert!((states[4] - 132.0).abs() < 0.1); // 1.1 * 120
    }

    #[test]
    fn test_build_transition_matrix() {
        let tracker = HmmBeatTracker::new(120.0, vec![0.0], 44100);
        let matrix = tracker.build_transition_matrix();

        assert_eq!(matrix.len(), 5);
        assert_eq!(matrix[0].len(), 5);

        // Self-transition should be highest (0.7)
        for (i, row) in matrix.iter().enumerate() {
            assert!(row[i] > 0.6, "Self-transition should be high");
        }

        // Adjacent transitions should be medium (0.15)
        for i in 0..4 {
            assert!(
                matrix[i][i + 1] > 0.1,
                "Adjacent transition should be medium"
            );
            assert!(
                matrix[i + 1][i] > 0.1,
                "Adjacent transition should be medium"
            );
        }

        // Distant transitions should be low (0.0)
        assert_eq!(matrix[0][4], 0.0);
        assert_eq!(matrix[4][0], 0.0);
    }

    #[test]
    fn test_nearest_onset_emission() {
        let onsets = vec![0.0, 0.5, 1.0, 1.5, 2.0]; // 120 BPM (0.5s intervals)
        let tracker = HmmBeatTracker::new(120.0, onsets, 44100);

        let (aligned_emission, aligned_distance) = tracker.nearest_onset_emission(1.0).unwrap();
        let (offset_emission, offset_distance) = tracker.nearest_onset_emission(1.04).unwrap();

        assert_eq!(aligned_emission, 1.0);
        assert_eq!(aligned_distance, 0.0);
        assert!((0.0..=1.0).contains(&offset_emission));
        assert!(offset_emission < aligned_emission);
        assert!((offset_distance - 0.04).abs() < 1e-5);
    }

    #[test]
    fn test_track_beats_basic() {
        // Create onsets at 120 BPM (0.5s intervals)
        let onsets = vec![0.0, 0.5, 1.0, 1.5, 2.0, 2.5];
        let tracker = HmmBeatTracker::new(120.0, onsets, 44100);

        let beats = tracker.track_beats().unwrap();

        assert!(!beats.is_empty(), "Should detect beats");
        assert!(beats.len() >= 3, "Should detect at least 3 beats");

        // Beats should be sorted by time
        for i in 1..beats.len() {
            assert!(
                beats[i].time_seconds > beats[i - 1].time_seconds,
                "Beats should be sorted by time"
            );
        }

        // Check confidence scores
        for beat in &beats {
            assert!(
                beat.confidence >= 0.0 && beat.confidence <= 1.0,
                "Confidence should be in [0, 1]"
            );
        }
    }

    #[test]
    fn test_track_beats_128bpm() {
        // Create onsets at 128 BPM (60/128 ≈ 0.469s intervals)
        let beat_interval = 60.0 / 128.0;
        let onsets: Vec<f32> = (0..6).map(|i| i as f32 * beat_interval).collect();

        let tracker = HmmBeatTracker::new(128.0, onsets, 44100);
        let beats = tracker.track_beats().unwrap();

        assert!(!beats.is_empty());

        // Check that beat intervals are approximately correct
        if beats.len() >= 2 {
            let interval = beats[1].time_seconds - beats[0].time_seconds;
            let expected_interval = 60.0 / 128.0;
            assert!(
                (interval - expected_interval).abs() < 0.1,
                "Beat interval should be close to expected"
            );
        }
    }

    #[test]
    fn test_track_beats_invalid_bpm() {
        let tracker = HmmBeatTracker::new(0.0, vec![0.0, 0.5], 44100);
        assert!(tracker.track_beats().is_err());

        let tracker = HmmBeatTracker::new(350.0, vec![0.0, 0.5], 44100);
        assert!(tracker.track_beats().is_err());

        let tracker = HmmBeatTracker::new(f32::NAN, vec![0.0, 0.5], 44100);
        assert!(tracker.track_beats().is_err());
    }

    #[test]
    fn test_track_beats_empty_onsets() {
        let tracker = HmmBeatTracker::new(120.0, vec![], 44100);
        assert!(tracker.track_beats().is_err());
    }

    #[test]
    fn test_track_beats_rejects_invalid_onsets_and_unbounded_horizon() {
        assert!(HmmBeatTracker::new(120.0, vec![0.0, f32::INFINITY], 44_100)
            .track_beats()
            .is_err());
        assert!(HmmBeatTracker::new(120.0, vec![0.5, 0.0], 44_100)
            .track_beats()
            .is_err());
        assert!(HmmBeatTracker::new(1e-9, vec![0.0, f32::MAX], 44_100)
            .track_beats()
            .is_err());
    }

    #[test]
    fn test_track_beats_single_onset() {
        let tracker = HmmBeatTracker::new(120.0, vec![0.5], 44100);
        // Single onset: should succeed with 0-1 beats (only one frame)
        let beats = tracker.track_beats().unwrap();
        assert!(
            beats.len() <= 1,
            "Single onset should produce at most 1 beat"
        );
    }

    #[test]
    fn test_viterbi_trace() {
        let onsets = vec![0.0, 0.5, 1.0, 1.5];
        let tracker = HmmBeatTracker::new(120.0, onsets, 44100);

        let state_bpms = tracker.build_state_space();
        let transition_matrix = tracker.build_transition_matrix();
        let trace = tracker
            .viterbi_trace(&state_bpms, &transition_matrix)
            .unwrap();

        // All states should be valid (0-4)
        assert!(!trace.steps.is_empty());
        assert!(trace.terminal_row < trace.max_steps);
        assert_eq!(trace.terminal_row + 1, trace.steps.len());
        for step in &trace.steps {
            assert!(step.state < 5, "State should be in range [0, 4]");
        }
    }

    #[test]
    fn test_extract_beats_from_trace() {
        let onsets = vec![0.0, 0.5, 1.0, 1.5, 2.0];
        let tracker = HmmBeatTracker::new(120.0, onsets, 44100);

        let state_bpms = tracker.build_state_space();
        let transition_matrix = tracker.build_transition_matrix();
        let trace = tracker
            .viterbi_trace(&state_bpms, &transition_matrix)
            .unwrap();

        let beats = tracker.extract_beats_from_trace(&trace).unwrap();

        assert!(!beats.is_empty());

        // Beats should be sorted
        for i in 1..beats.len() {
            assert!(beats[i].time_seconds > beats[i - 1].time_seconds);
        }
    }

    #[test]
    fn extraction_merges_close_cells_by_confidence() {
        let tracker = HmmBeatTracker::new(120.0, vec![0.0, 0.03, 0.5], 44_100);
        let trace = ViterbiTrace {
            steps: vec![
                PathStep {
                    state: 2,
                    expected_time: 0.0,
                    emission_probability: 0.6,
                    onset_distance: 0.0,
                },
                PathStep {
                    state: 2,
                    expected_time: 0.03,
                    emission_probability: 0.9,
                    onset_distance: 0.0,
                },
                PathStep {
                    state: 2,
                    expected_time: 0.5,
                    emission_probability: 1.0,
                    onset_distance: 0.0,
                },
            ],
            terminal_row: 2,
            terminal_state: 2,
            max_steps: 3,
        };

        let beats = tracker.extract_beats_from_trace(&trace).unwrap();
        assert_eq!(beats.len(), 2);
        assert_eq!(beats[0].time_seconds, 0.03);
        assert!(beats[0].confidence > 0.9);
        assert_eq!(beats[1].time_seconds, 0.5);
    }

    #[test]
    fn test_cumulative_emissions_differentiate_states() {
        // Onsets at 108 BPM (state 0 = 0.9 * 120). The slower state should
        // align better with these onsets than the faster states.
        let interval_108 = 60.0 / 108.0; // ~0.556s
        let onsets: Vec<f32> = (0..10).map(|i| i as f32 * interval_108).collect();
        let tracker = HmmBeatTracker::new(120.0, onsets, 44100);
        let mut state_0_time = 0.0;
        let mut state_4_time = 0.0;
        let mut total_state_0 = 0.0;
        let mut total_state_4 = 0.0;
        for _ in 0..10 {
            total_state_0 += tracker.nearest_onset_emission(state_0_time).unwrap().0;
            total_state_4 += tracker.nearest_onset_emission(state_4_time).unwrap().0;
            state_0_time += 60.0 / 108.0;
            state_4_time += 60.0 / 132.0;
        }
        assert!(
            total_state_0 > total_state_4,
            "State 0 (108 BPM) should score higher than state 4 (132 BPM) \
             for 108 BPM onsets: {total_state_0:.3} vs {total_state_4:.3}"
        );
    }

    #[test]
    fn test_viterbi_favours_matching_state() {
        // Onsets at 108 BPM (state 0 = 0.9 * 120). The Viterbi path should
        // select state 0 or 1 as the dominant state.
        let interval_108 = 60.0 / 108.0;
        let onsets: Vec<f32> = (0..16).map(|i| i as f32 * interval_108).collect();
        let tracker = HmmBeatTracker::new(120.0, onsets, 44100);

        let state_bpms = tracker.build_state_space();
        let transition_matrix = tracker.build_transition_matrix();
        let trace = tracker
            .viterbi_trace(&state_bpms, &transition_matrix)
            .unwrap();

        // Find the most frequent state in the path
        let mut counts = [0usize; NUM_STATES];
        for step in &trace.steps {
            counts[step.state] += 1;
        }
        let dominant_state = counts
            .iter()
            .enumerate()
            .max_by_key(|&(_, &c)| c)
            .map(|(s, _)| s)
            .unwrap();
        assert!(
            dominant_state <= 1,
            "Dominant state should be 0 or 1 (slow) for 108 BPM onsets, \
             got state {dominant_state}, counts={counts:?}, path={:?}",
            trace.steps
        );
    }

    #[test]
    fn known_state_path_advances_by_destination_intervals() {
        let state_bpms = [108.0_f32, 114.0, 120.0, 126.0, 132.0];
        let states = [2_usize, 2, 3, 3, 4, 4, 4];
        let mut times = vec![0.25_f32];
        for state in states.iter().copied().skip(1) {
            let next = HmmBeatTracker::next_expected_time(
                times.last().copied().unwrap(),
                state_bpms[state],
            )
            .unwrap();
            times.push(next);
        }

        for (index, pair) in times.windows(2).enumerate() {
            let destination_state = states[index + 1];
            let expected = pair[0] + 60.0 / state_bpms[destination_state];
            assert!((pair[1] - expected).abs() < 1e-6);
        }
    }

    #[test]
    fn fastest_state_horizon_reaches_sustained_tail() {
        let fast_interval = 60.0 / 132.0;
        let onsets: Vec<f32> = (0..32).map(|index| index as f32 * fast_interval).collect();
        let tracker = HmmBeatTracker::new(120.0, onsets.clone(), 44_100);
        let state_bpms = tracker.build_state_space();
        let transition_matrix = tracker.build_transition_matrix();
        let trace = tracker
            .viterbi_trace(&state_bpms, &transition_matrix)
            .unwrap();
        let old_nominal_frames =
            ((onsets.last().unwrap() - onsets[0]) / (60.0 / 120.0)).ceil() as usize + 1;

        assert!(trace.max_steps > old_nominal_frames);
        assert!(trace.terminal_row + 1 > old_nominal_frames);
        assert!(
            (trace.steps.last().unwrap().expected_time - onsets.last().unwrap()).abs()
                <= fast_interval
        );
    }

    #[test]
    fn variable_tempo_output_is_cumulative_strict_and_onset_unique() {
        let state_bpms = [108.0_f32, 114.0, 120.0, 126.0, 132.0];
        let states = [
            0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 4, 4, 4, 4,
        ];
        let mut onsets = vec![0.0_f32];
        for state in states.iter().copied().skip(1) {
            let next = onsets.last().copied().unwrap() + 60.0 / state_bpms[state];
            onsets.push(next);
        }

        let tracker = HmmBeatTracker::new(120.0, onsets.clone(), 44_100);
        let beats = tracker.track_beats().unwrap();

        assert!(beats.len() >= onsets.len().saturating_sub(1));
        assert!(beats
            .windows(2)
            .all(|pair| pair[0].time_seconds < pair[1].time_seconds));

        let mut used_onsets = vec![false; onsets.len()];
        for beat in &beats {
            assert!(beat.time_seconds.is_finite() && beat.time_seconds >= 0.0);
            let (nearest_index, nearest_distance) = onsets
                .iter()
                .enumerate()
                .filter(|(index, _)| !used_onsets[*index])
                .map(|(index, onset)| (index, (*onset - beat.time_seconds).abs()))
                .min_by(|left, right| left.1.total_cmp(&right.1))
                .expect("each output beat should have an unused source onset");
            assert!(
                nearest_distance <= TIMING_TOLERANCE_S,
                "beat at {:.6}s should align uniquely to an onset, nearest distance {:.6}s",
                beat.time_seconds,
                nearest_distance
            );
            used_onsets[nearest_index] = true;
        }

        let maximum_supported_interval = 60.0 / state_bpms[0];
        assert!(beats.windows(2).all(|pair| {
            pair[1].time_seconds - pair[0].time_seconds <= maximum_supported_interval * 1.05
        }));
        assert!(
            (beats.last().unwrap().time_seconds - onsets.last().unwrap()).abs()
                <= 60.0 / state_bpms[4],
            "fast tail should reach the final pulse without nominal-horizon truncation"
        );
    }
}
