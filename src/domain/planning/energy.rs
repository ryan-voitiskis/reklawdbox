//! Energy-curve and BPM-trajectory policy.

use super::{EnergyCurve, EnergyCurvePreset, EnergyPhase};

const WARMUP_PHASE_END: f64 = 0.15;
const BUILD_PHASE_END: f64 = 0.45;
const PEAK_PHASE_END: f64 = 0.75;
const PEAKONLY_BUILD_END: f64 = 0.10;
const PEAKONLY_RELEASE_END: f64 = 0.85;

const BPM_PROXY_FLOOR: f64 = 95.0;
const BPM_PROXY_RANGE: f64 = 50.0;

const DANCEABILITY_MAX: f64 = 3.0;
const LOUDNESS_FLOOR_LUFS: f64 = -30.0;
const LOUDNESS_RANGE_LUFS: f64 = 30.0;
const ONSET_RATE_MAX: f64 = 10.0;

const ENERGY_W_DANCE: f64 = 0.4;
const ENERGY_W_LOUDNESS: f64 = 0.3;
const ENERGY_W_ONSET: f64 = 0.3;

pub(crate) fn resolve_energy_curve(
    energy_curve: Option<&EnergyCurve>,
    target_tracks: usize,
) -> Result<Vec<EnergyPhase>, String> {
    if target_tracks == 0 {
        return Err("target_tracks must be at least 1".to_string());
    }

    match energy_curve {
        Some(EnergyCurve::Custom(phases)) => {
            if phases.len() != target_tracks {
                return Err(format!(
                    "custom phase array length ({}) must match target_tracks ({target_tracks})",
                    phases.len()
                ));
            }
            Ok(phases.clone())
        }
        Some(EnergyCurve::Preset(preset)) => Ok((0..target_tracks)
            .map(|position| preset_energy_phase(*preset, position, target_tracks))
            .collect()),
        None => Ok((0..target_tracks)
            .map(|position| {
                preset_energy_phase(
                    EnergyCurvePreset::WarmupBuildPeakRelease,
                    position,
                    target_tracks,
                )
            })
            .collect()),
    }
}

fn preset_energy_phase(preset: EnergyCurvePreset, position: usize, total: usize) -> EnergyPhase {
    let fraction = if total == 0 {
        0.0
    } else {
        position as f64 / total as f64
    };
    match preset {
        EnergyCurvePreset::WarmupBuildPeakRelease => {
            if fraction < WARMUP_PHASE_END {
                EnergyPhase::Warmup
            } else if fraction < BUILD_PHASE_END {
                EnergyPhase::Build
            } else if fraction < PEAK_PHASE_END {
                EnergyPhase::Peak
            } else {
                EnergyPhase::Release
            }
        }
        EnergyCurvePreset::FlatEnergy => EnergyPhase::Peak,
        EnergyCurvePreset::PeakOnly => {
            if fraction < PEAKONLY_BUILD_END {
                EnergyPhase::Build
            } else if fraction < PEAKONLY_RELEASE_END {
                EnergyPhase::Peak
            } else {
                EnergyPhase::Release
            }
        }
    }
}

pub(crate) fn compute_bpm_trajectory(
    phases: &[EnergyPhase],
    start_bpm: f64,
    end_bpm: f64,
) -> Vec<f64> {
    if phases.is_empty() {
        return Vec::new();
    }

    let build_start = phases.iter().position(|p| *p == EnergyPhase::Build);
    let build_end = phases.iter().rposition(|p| *p == EnergyPhase::Build);
    let release_start = phases.iter().position(|p| *p == EnergyPhase::Release);
    let release_end = phases.iter().rposition(|p| *p == EnergyPhase::Release);

    phases
        .iter()
        .enumerate()
        .map(|(i, phase)| match phase {
            EnergyPhase::Warmup => start_bpm,
            EnergyPhase::Build => {
                if let (Some(bs), Some(be)) = (build_start, build_end) {
                    if bs == be {
                        (start_bpm + end_bpm) / 2.0
                    } else {
                        let progress = (i - bs) as f64 / (be - bs) as f64;
                        start_bpm + (end_bpm - start_bpm) * progress
                    }
                } else {
                    start_bpm
                }
            }
            EnergyPhase::Peak => end_bpm,
            EnergyPhase::Release => {
                if let (Some(rs), Some(re)) = (release_start, release_end) {
                    if rs == re {
                        (start_bpm + end_bpm) / 2.0
                    } else {
                        let progress = (i - rs) as f64 / (re - rs) as f64;
                        end_bpm + (start_bpm - end_bpm) * progress
                    }
                } else {
                    end_bpm
                }
            }
        })
        .collect()
}

pub(crate) fn compute_track_energy(
    essentia_components: Option<(Option<f64>, Option<f64>, Option<f64>)>,
    bpm: f64,
) -> f64 {
    let bpm_proxy = ((bpm - BPM_PROXY_FLOOR) / BPM_PROXY_RANGE).clamp(0.0, 1.0);
    let Some((danceability, loudness_integrated, onset_rate)) = essentia_components else {
        return bpm_proxy;
    };

    match (danceability, loudness_integrated, onset_rate) {
        (Some(dance), Some(loudness), Some(onset)) => {
            let normalized_dance = (dance / DANCEABILITY_MAX).clamp(0.0, 1.0);
            let normalized_loudness =
                ((loudness - LOUDNESS_FLOOR_LUFS) / LOUDNESS_RANGE_LUFS).clamp(0.0, 1.0);
            let onset_rate_normalized = (onset / ONSET_RATE_MAX).clamp(0.0, 1.0);
            ((ENERGY_W_DANCE * normalized_dance)
                + (ENERGY_W_LOUDNESS * normalized_loudness)
                + (ENERGY_W_ONSET * onset_rate_normalized))
                .clamp(0.0, 1.0)
        }
        _ => bpm_proxy,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planning_energy_curve_preserves_default_phase_order() {
        assert_eq!(
            resolve_energy_curve(None, 4).unwrap(),
            [
                EnergyPhase::Warmup,
                EnergyPhase::Build,
                EnergyPhase::Peak,
                EnergyPhase::Release,
            ]
        );
    }
}
