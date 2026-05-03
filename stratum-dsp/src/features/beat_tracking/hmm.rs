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
        // Validate inputs
        if self.bpm_estimate <= EPSILON || self.bpm_estimate > 300.0 {
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

        log::debug!(
            "Tracking beats with HMM: BPM={:.2}, {} onsets",
            self.bpm_estimate,
            self.onsets.len()
        );

        // Build state space (BPM variations)
        let state_bpms = self.build_state_space();

        // Build transition probability matrix
        let transition_matrix = self.build_transition_matrix();

        // Compute emission probabilities for each time frame
        let emission_matrix = self.compute_emission_probabilities(&state_bpms)?;

        // Run Viterbi algorithm
        let best_path = self.viterbi_forward_pass(&transition_matrix, &emission_matrix)?;

        // Extract beats from best path
        let beats = self.extract_beats_from_path(&best_path, &state_bpms, &emission_matrix)?;

        log::debug!("HMM beat tracking complete: {} beats detected", beats.len());

        Ok(beats)
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

    /// Compute emission probabilities for each time frame
    ///
    /// For each time frame and each state, compute the probability of observing
    /// the onsets given that state's BPM. Uses Gaussian decay based on distance
    /// to nearest onset.
    ///
    /// Formula: emission[t][s] = exp(-distance² / (2 * σ²))
    /// where σ = EMISSION_SIGMA (typically 25ms)
    ///
    /// Returns T × NUM_STATES matrix where T is number of time frames
    fn compute_emission_probabilities(
        &self,
        state_bpms: &[f32],
    ) -> Result<Vec<Vec<f32>>, AnalysisError> {
        if self.onsets.is_empty() {
            return Err(AnalysisError::InvalidInput(
                "No onsets for emission computation".to_string(),
            ));
        }

        // Determine time range: from first onset to last onset
        let start_time = self.onsets[0];
        let end_time = self.onsets[self.onsets.len() - 1];

        // Create time frames: one frame per expected beat interval
        // Use nominal BPM to determine frame spacing (for indexing only)
        let nominal_beat_interval = 60.0 / self.bpm_estimate;
        let num_frames = ((end_time - start_time) / nominal_beat_interval).ceil() as usize + 1;

        if num_frames == 0 {
            return Err(AnalysisError::ProcessingError(
                "Cannot compute emissions: invalid time range".to_string(),
            ));
        }

        let sigma_sq = EMISSION_SIGMA * EMISSION_SIGMA;
        if sigma_sq <= EPSILON {
            return Err(AnalysisError::NumericalError(
                "Emission sigma too small".to_string(),
            ));
        }

        // Use a small floor so that out-of-range frames get a low but non-zero
        // emission rather than 0, avoiding log(0) in the log-space Viterbi.
        let mut emission_matrix = vec![vec![EMISSION_FLOOR; NUM_STATES]; num_frames];

        // For each time frame
        #[allow(clippy::needless_range_loop)]
        for t in 0..num_frames {
            // For each state, compute expected beat time using that state's BPM.
            // Slow states may project past end_time for high frame indices, and
            // fast states may not cover the full range — the emission_floor
            // ensures these out-of-range frames don't dominate or vanish.
            for s in 0..NUM_STATES {
                let state_beat_interval = 60.0 / state_bpms[s];
                let expected_beat_time = start_time + (t as f32 * state_beat_interval);

                // Skip frames that project well beyond the observable onset range
                if expected_beat_time > end_time + state_beat_interval {
                    continue; // keeps EMISSION_FLOOR
                }

                // Find distance to nearest onset
                let mut min_distance = f32::INFINITY;
                for &onset_time in &self.onsets {
                    let distance = (onset_time - expected_beat_time).abs();
                    if distance < min_distance {
                        min_distance = distance;
                    }
                }

                // Compute emission probability using Gaussian decay
                // emission = exp(-distance² / (2 * σ²))
                let distance_sq = min_distance * min_distance;
                let emission = (-distance_sq / (2.0 * sigma_sq)).exp().max(EMISSION_FLOOR);
                emission_matrix[t][s] = emission;
            }
        }

        Ok(emission_matrix)
    }

    /// Run Viterbi forward pass
    ///
    /// Computes the best path probability for each state at each time frame.
    /// Also stores backpointers for backtracking.
    ///
    /// Returns (viterbi_matrix, backpointer_matrix) where:
    /// - viterbi_matrix[t][s] = best path probability ending at state s at time t
    /// - backpointer_matrix[t][s] = previous state in best path
    fn viterbi_forward_pass(
        &self,
        transition_matrix: &[Vec<f32>],
        emission_matrix: &[Vec<f32>],
    ) -> Result<Vec<usize>, AnalysisError> {
        let num_frames = emission_matrix.len();

        if num_frames == 0 {
            return Err(AnalysisError::ProcessingError(
                "Cannot run Viterbi: no time frames".to_string(),
            ));
        }

        // Work in log-space to avoid probability underflow on long sequences.
        let mut log_viterbi = vec![vec![f32::NEG_INFINITY; NUM_STATES]; num_frames];
        let mut backpointer = vec![vec![0; NUM_STATES]; num_frames];

        // Pre-compute log transition probabilities (0 entries -> NEG_INFINITY)
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

        // Initialization: uniform prior
        let log_initial = (1.0 / NUM_STATES as f32).ln();
        for s in 0..NUM_STATES {
            log_viterbi[0][s] = log_initial + emission_matrix[0][s].max(EPSILON).ln();
        }

        // Forward pass in log-space: log(a*b) = log(a) + log(b)
        for t in 1..num_frames {
            for s in 0..NUM_STATES {
                let mut best_log_prob = f32::NEG_INFINITY;
                let mut best_prev_state = 0;

                for prev_s in 0..NUM_STATES {
                    let log_prob = log_viterbi[t - 1][prev_s] + log_transition[prev_s][s];
                    if log_prob > best_log_prob {
                        best_log_prob = log_prob;
                        best_prev_state = prev_s;
                    }
                }

                log_viterbi[t][s] = best_log_prob + emission_matrix[t][s].max(EPSILON).ln();
                backpointer[t][s] = best_prev_state;
            }
        }

        // Backtracking: find best final state
        let mut best_path = vec![0; num_frames];
        let mut best_final_log_prob = f32::NEG_INFINITY;
        let mut best_final_state = 0;

        for (s, &log_prob) in log_viterbi[num_frames - 1].iter().enumerate() {
            if log_prob > best_final_log_prob {
                best_final_log_prob = log_prob;
                best_final_state = s;
            }
        }

        best_path[num_frames - 1] = best_final_state;

        for t in (0..num_frames - 1).rev() {
            best_path[t] = backpointer[t + 1][best_path[t + 1]];
        }

        Ok(best_path)
    }

    /// Extract beats from Viterbi path
    ///
    /// Converts the state sequence from Viterbi algorithm into actual beat positions
    /// by finding times where emission probability is high.
    ///
    /// Returns vector of beat positions with confidence scores
    fn extract_beats_from_path(
        &self,
        best_path: &[usize],
        state_bpms: &[f32],
        emission_matrix: &[Vec<f32>],
    ) -> Result<Vec<BeatPosition>, AnalysisError> {
        if best_path.is_empty() || emission_matrix.is_empty() {
            return Err(AnalysisError::ProcessingError(
                "Cannot extract beats: empty path or emission matrix".to_string(),
            ));
        }

        let start_time = self.onsets[0];

        let mut beats = Vec::new();

        // For each frame in the path
        for (t, &state) in best_path.iter().enumerate() {
            let emission_prob = emission_matrix[t][state];

            // Only add beat if emission probability is above threshold
            if emission_prob > EMISSION_THRESHOLD {
                // Use the state's BPM to compute beat time, matching emission computation
                let state_beat_interval = 60.0 / state_bpms[state];
                let beat_time = start_time + (t as f32 * state_beat_interval);

                // Find nearest onset to compute confidence
                let mut min_distance = f32::INFINITY;
                for &onset_time in &self.onsets {
                    let distance = (onset_time - beat_time).abs();
                    if distance < min_distance {
                        min_distance = distance;
                    }
                }

                // Confidence based on alignment: higher if closer to onset
                // Normalize to [0, 1] using timing tolerance
                let alignment_score = if min_distance < TIMING_TOLERANCE_S {
                    1.0 - (min_distance / TIMING_TOLERANCE_S)
                } else {
                    0.0
                };

                // Combine emission probability and alignment score
                let confidence = (emission_prob * 0.7 + alignment_score * 0.3).min(1.0);

                beats.push(BeatPosition {
                    beat_index: 0, // Will be set by beat grid generation
                    time_seconds: beat_time,
                    confidence,
                });
            }
        }

        // Sort by time (should already be sorted, but ensure it)
        beats.sort_by(|a, b| a.time_seconds.partial_cmp(&b.time_seconds).unwrap());

        Ok(beats)
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
    fn test_compute_emission_probabilities() {
        let onsets = vec![0.0, 0.5, 1.0, 1.5, 2.0]; // 120 BPM (0.5s intervals)
        let tracker = HmmBeatTracker::new(120.0, onsets.clone(), 44100);
        let state_bpms = tracker.build_state_space();

        let emissions = tracker.compute_emission_probabilities(&state_bpms).unwrap();

        assert!(!emissions.is_empty());
        assert_eq!(emissions[0].len(), 5); // 5 states

        // Emissions should be in [0, 1] range
        for row in &emissions {
            for &prob in row {
                assert!(
                    (0.0..=1.0).contains(&prob),
                    "Emission probability should be in [0, 1]"
                );
            }
        }
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
    }

    #[test]
    fn test_track_beats_empty_onsets() {
        let tracker = HmmBeatTracker::new(120.0, vec![], 44100);
        assert!(tracker.track_beats().is_err());
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
    fn test_viterbi_forward_pass() {
        let onsets = vec![0.0, 0.5, 1.0, 1.5];
        let tracker = HmmBeatTracker::new(120.0, onsets, 44100);

        let state_bpms = tracker.build_state_space();
        let transition_matrix = tracker.build_transition_matrix();
        let emission_matrix = tracker.compute_emission_probabilities(&state_bpms).unwrap();

        let best_path = tracker
            .viterbi_forward_pass(&transition_matrix, &emission_matrix)
            .unwrap();

        assert!(!best_path.is_empty());
        assert_eq!(best_path.len(), emission_matrix.len());

        // All states should be valid (0-4)
        for &state in &best_path {
            assert!(state < 5, "State should be in range [0, 4]");
        }
    }

    #[test]
    fn test_extract_beats_from_path() {
        let onsets = vec![0.0, 0.5, 1.0, 1.5, 2.0];
        let tracker = HmmBeatTracker::new(120.0, onsets, 44100);

        let state_bpms = tracker.build_state_space();
        let transition_matrix = tracker.build_transition_matrix();
        let emission_matrix = tracker.compute_emission_probabilities(&state_bpms).unwrap();
        let best_path = tracker
            .viterbi_forward_pass(&transition_matrix, &emission_matrix)
            .unwrap();

        let beats = tracker
            .extract_beats_from_path(&best_path, &state_bpms, &emission_matrix)
            .unwrap();

        assert!(!beats.is_empty());

        // Beats should be sorted
        for i in 1..beats.len() {
            assert!(beats[i].time_seconds > beats[i - 1].time_seconds);
        }
    }

    #[test]
    fn test_emissions_differentiate_states() {
        // Onsets at 108 BPM (state 0 = 0.9 * 120). The slower state should
        // align better with these onsets than the faster states.
        let interval_108 = 60.0 / 108.0; // ~0.556s
        let onsets: Vec<f32> = (0..10).map(|i| i as f32 * interval_108).collect();
        let tracker = HmmBeatTracker::new(120.0, onsets, 44100);
        let state_bpms = tracker.build_state_space();

        let emissions = tracker.compute_emission_probabilities(&state_bpms).unwrap();

        // Over many frames, state 0 (108 BPM) should accumulate higher total
        // emission probability than state 4 (132 BPM) since the onsets were
        // generated at 108 BPM.
        let total_state_0: f32 = emissions.iter().map(|row| row[0]).sum();
        let total_state_4: f32 = emissions.iter().map(|row| row[4]).sum();
        assert!(
            total_state_0 > total_state_4,
            "State 0 (108 BPM) should score higher than state 4 (132 BPM) \
             for 108 BPM onsets: {total_state_0:.3} vs {total_state_4:.3}"
        );
    }

    #[test]
    fn test_viterbi_favours_matching_state() {
        // Onsets at 108 BPM (state 0 = 0.9 * 120). The Viterbi path should
        // select state 0 or 1 as the dominant state. We allow state 1 (114 BPM)
        // because the frame grid uses the nominal BPM for indexing, which causes
        // slight misalignment for the slowest state at later frames.
        let interval_108 = 60.0 / 108.0;
        let onsets: Vec<f32> = (0..16).map(|i| i as f32 * interval_108).collect();
        let tracker = HmmBeatTracker::new(120.0, onsets, 44100);

        let state_bpms = tracker.build_state_space();
        let transition_matrix = tracker.build_transition_matrix();
        let emission_matrix = tracker.compute_emission_probabilities(&state_bpms).unwrap();
        let best_path = tracker
            .viterbi_forward_pass(&transition_matrix, &emission_matrix)
            .unwrap();

        // Find the most frequent state in the path
        let mut counts = [0usize; NUM_STATES];
        for &s in &best_path {
            counts[s] += 1;
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
             got state {dominant_state}, counts={counts:?}, path={best_path:?}"
        );
    }
}
