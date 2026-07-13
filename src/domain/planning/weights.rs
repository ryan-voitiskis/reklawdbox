//! Built-in planning weights and pure normalization policy.

use super::{PoolPreset, PoolWeights, PriorityWeights, SequencingPriority};

pub(crate) fn priority_weights(priority: SequencingPriority) -> PriorityWeights {
    match priority {
        SequencingPriority::Balanced => PriorityWeights {
            key: 0.30,
            bpm: 0.20,
            energy: 0.18,
            genre: 0.17,
            brightness: 0.08,
            rhythm: 0.07,
        },
        SequencingPriority::Harmonic => PriorityWeights {
            key: 0.48,
            bpm: 0.18,
            energy: 0.12,
            genre: 0.08,
            brightness: 0.08,
            rhythm: 0.06,
        },
        SequencingPriority::Energy => PriorityWeights {
            key: 0.12,
            bpm: 0.18,
            energy: 0.42,
            genre: 0.12,
            brightness: 0.08,
            rhythm: 0.08,
        },
        SequencingPriority::Genre => PriorityWeights {
            key: 0.18,
            bpm: 0.18,
            energy: 0.12,
            genre: 0.38,
            brightness: 0.08,
            rhythm: 0.06,
        },
    }
}

pub(crate) fn pool_weights(preset: PoolPreset) -> PoolWeights {
    let weights = match preset {
        PoolPreset::Balanced => PoolWeights {
            bpm: 0.25,
            energy: 0.20,
            timbral: 0.18,
            key: 0.12,
            genre: 0.10,
            brightness: 0.08,
            rhythm: 0.07,
        },
        PoolPreset::Timbral => PoolWeights {
            bpm: 0.20,
            energy: 0.15,
            timbral: 0.35,
            key: 0.10,
            genre: 0.05,
            brightness: 0.08,
            rhythm: 0.07,
        },
    };
    debug_assert!(
        (weights.bpm
            + weights.energy
            + weights.timbral
            + weights.key
            + weights.genre
            + weights.brightness
            + weights.rhythm
            - 1.0)
            .abs()
            < 1e-10,
        "pool weights must sum to 1.0"
    );
    weights
}

pub(crate) fn renormalize_transition(weights: &mut PriorityWeights) -> Result<(), String> {
    let fields = [
        weights.key,
        weights.bpm,
        weights.energy,
        weights.genre,
        weights.brightness,
        weights.rhythm,
    ];
    let sum = validate_and_sum(&fields)?;
    weights.key /= sum;
    weights.bpm /= sum;
    weights.energy /= sum;
    weights.genre /= sum;
    weights.brightness /= sum;
    weights.rhythm /= sum;
    Ok(())
}

pub(crate) fn renormalize_pool(weights: &mut PoolWeights) -> Result<(), String> {
    let fields = [
        weights.bpm,
        weights.energy,
        weights.timbral,
        weights.key,
        weights.genre,
        weights.brightness,
        weights.rhythm,
    ];
    let sum = validate_and_sum(&fields)?;
    weights.bpm /= sum;
    weights.energy /= sum;
    weights.timbral /= sum;
    weights.key /= sum;
    weights.genre /= sum;
    weights.brightness /= sum;
    weights.rhythm /= sum;
    Ok(())
}

fn validate_and_sum(fields: &[f64]) -> Result<f64, String> {
    if let Some(negative) = fields.iter().find(|&&value| value < 0.0) {
        return Err(format!(
            "Negative weight ({negative}) — all weights must be >= 0"
        ));
    }
    let sum: f64 = fields.iter().sum();
    if sum <= f64::EPSILON {
        return Err("All weights are zero — at least one must be positive".into());
    }
    Ok(sum)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planning_weights_preserve_balanced_pool_sum() {
        let weights = pool_weights(PoolPreset::Balanced);
        let sum = weights.bpm
            + weights.energy
            + weights.timbral
            + weights.key
            + weights.genre
            + weights.brightness
            + weights.rhythm;
        assert!((sum - 1.0).abs() < 1e-10);
    }
}
