use rusqlite::Connection;

use super::params::*;
use super::scoring::{PoolWeights, PriorityWeights};

pub(super) fn resolve_transition_weights(
    spec: Option<&TransitionWeightSpec>,
    store: &Connection,
) -> Result<PriorityWeights, String> {
    let Some(spec) = spec else {
        return Ok(super::scoring::priority_weights(
            SequencingPriority::Balanced,
        ));
    };

    match spec {
        TransitionWeightSpec::Named(name) => resolve_transition_named(name, store),
        TransitionWeightSpec::WithOverrides { preset, overrides } => {
            let mut base = resolve_transition_named(preset, store)?;
            if let Some(overrides) = overrides {
                apply_transition_overrides(&mut base, overrides);
                renormalize_transition(&mut base)?;
            }
            Ok(base)
        }
        TransitionWeightSpec::Custom(input) => {
            let mut w = transition_input_to_weights(input);
            renormalize_transition(&mut w)?;
            Ok(w)
        }
    }
}

fn resolve_transition_named(name: &str, store: &Connection) -> Result<PriorityWeights, String> {
    match name {
        "balanced" => {
            return Ok(super::scoring::priority_weights(
                SequencingPriority::Balanced,
            ));
        }
        "harmonic" => {
            return Ok(super::scoring::priority_weights(
                SequencingPriority::Harmonic,
            ));
        }
        "energy" => return Ok(super::scoring::priority_weights(SequencingPriority::Energy)),
        "genre" => return Ok(super::scoring::priority_weights(SequencingPriority::Genre)),
        _ => {}
    }

    let json = crate::store::get_weight_preset(store, name, "transition")
        .map_err(|e| format!("DB error: {e}"))?
        .ok_or_else(|| {
            format!(
                "Unknown transition preset '{name}'. Built-in: balanced, harmonic, energy, genre"
            )
        })?;

    let input: TransitionWeightInput =
        serde_json::from_str(&json).map_err(|e| format!("Invalid saved preset: {e}"))?;
    let mut w = transition_input_to_weights(&input);
    renormalize_transition(&mut w)?;
    Ok(w)
}

pub(super) fn apply_transition_overrides(
    w: &mut PriorityWeights,
    overrides: &TransitionWeightInput,
) {
    if let Some(v) = overrides.key {
        w.key = v;
    }
    if let Some(v) = overrides.bpm {
        w.bpm = v;
    }
    if let Some(v) = overrides.energy {
        w.energy = v;
    }
    if let Some(v) = overrides.genre {
        w.genre = v;
    }
    if let Some(v) = overrides.brightness {
        w.brightness = v;
    }
    if let Some(v) = overrides.rhythm {
        w.rhythm = v;
    }
}

pub(super) fn transition_input_to_weights(input: &TransitionWeightInput) -> PriorityWeights {
    let base = super::scoring::priority_weights(SequencingPriority::Balanced);
    PriorityWeights {
        key: input.key.unwrap_or(base.key),
        bpm: input.bpm.unwrap_or(base.bpm),
        energy: input.energy.unwrap_or(base.energy),
        genre: input.genre.unwrap_or(base.genre),
        brightness: input.brightness.unwrap_or(base.brightness),
        rhythm: input.rhythm.unwrap_or(base.rhythm),
    }
}

pub(super) fn renormalize_transition(w: &mut PriorityWeights) -> Result<(), String> {
    let fields = [w.key, w.bpm, w.energy, w.genre, w.brightness, w.rhythm];
    if let Some(neg) = fields.iter().find(|&&v| v < 0.0) {
        return Err(format!(
            "Negative weight ({neg}) — all weights must be >= 0"
        ));
    }
    let sum: f64 = fields.iter().sum();
    if sum <= f64::EPSILON {
        return Err("All weights are zero — at least one must be positive".into());
    }
    w.key /= sum;
    w.bpm /= sum;
    w.energy /= sum;
    w.genre /= sum;
    w.brightness /= sum;
    w.rhythm /= sum;
    Ok(())
}

pub(super) fn resolve_pool_weights(
    spec: Option<&PoolWeightSpec>,
    store: &Connection,
) -> Result<PoolWeights, String> {
    let Some(spec) = spec else {
        return Ok(super::scoring::pool_weights(PoolPreset::Balanced));
    };

    match spec {
        PoolWeightSpec::Named(name) => resolve_pool_named(name, store),
        PoolWeightSpec::WithOverrides { preset, overrides } => {
            let mut base = resolve_pool_named(preset, store)?;
            if let Some(overrides) = overrides {
                apply_pool_overrides(&mut base, overrides);
                renormalize_pool(&mut base)?;
            }
            Ok(base)
        }
        PoolWeightSpec::Custom(input) => {
            let mut w = pool_input_to_weights(input);
            renormalize_pool(&mut w)?;
            Ok(w)
        }
    }
}

fn resolve_pool_named(name: &str, store: &Connection) -> Result<PoolWeights, String> {
    match name {
        "balanced" => return Ok(super::scoring::pool_weights(PoolPreset::Balanced)),
        "timbral" => return Ok(super::scoring::pool_weights(PoolPreset::Timbral)),
        _ => {}
    }

    let json = crate::store::get_weight_preset(store, name, "pool")
        .map_err(|e| format!("DB error: {e}"))?
        .ok_or_else(|| format!("Unknown pool preset '{name}'. Built-in: balanced, timbral"))?;

    let input: PoolWeightInput =
        serde_json::from_str(&json).map_err(|e| format!("Invalid saved preset: {e}"))?;
    let mut w = pool_input_to_weights(&input);
    renormalize_pool(&mut w)?;
    Ok(w)
}

pub(super) fn apply_pool_overrides(w: &mut PoolWeights, overrides: &PoolWeightInput) {
    if let Some(v) = overrides.bpm {
        w.bpm = v;
    }
    if let Some(v) = overrides.energy {
        w.energy = v;
    }
    if let Some(v) = overrides.timbral {
        w.timbral = v;
    }
    if let Some(v) = overrides.key {
        w.key = v;
    }
    if let Some(v) = overrides.genre {
        w.genre = v;
    }
    if let Some(v) = overrides.brightness {
        w.brightness = v;
    }
    if let Some(v) = overrides.rhythm {
        w.rhythm = v;
    }
}

pub(super) fn pool_input_to_weights(input: &PoolWeightInput) -> PoolWeights {
    let base = super::scoring::pool_weights(PoolPreset::Balanced);
    PoolWeights {
        bpm: input.bpm.unwrap_or(base.bpm),
        energy: input.energy.unwrap_or(base.energy),
        timbral: input.timbral.unwrap_or(base.timbral),
        key: input.key.unwrap_or(base.key),
        genre: input.genre.unwrap_or(base.genre),
        brightness: input.brightness.unwrap_or(base.brightness),
        rhythm: input.rhythm.unwrap_or(base.rhythm),
    }
}

pub(super) fn renormalize_pool(w: &mut PoolWeights) -> Result<(), String> {
    let fields = [
        w.bpm,
        w.energy,
        w.timbral,
        w.key,
        w.genre,
        w.brightness,
        w.rhythm,
    ];
    if let Some(neg) = fields.iter().find(|&&v| v < 0.0) {
        return Err(format!(
            "Negative weight ({neg}) — all weights must be >= 0"
        ));
    }
    let sum: f64 = fields.iter().sum();
    if sum <= f64::EPSILON {
        return Err("All weights are zero — at least one must be positive".into());
    }
    w.bpm /= sum;
    w.energy /= sum;
    w.timbral /= sum;
    w.key /= sum;
    w.genre /= sum;
    w.brightness /= sum;
    w.rhythm /= sum;
    Ok(())
}

pub(super) fn transition_weights_to_json(w: &PriorityWeights) -> serde_json::Value {
    serde_json::json!({
        "key": w.key, "bpm": w.bpm, "energy": w.energy,
        "genre": w.genre, "brightness": w.brightness, "rhythm": w.rhythm,
    })
}

pub(super) fn pool_weights_to_json(w: &PoolWeights) -> serde_json::Value {
    serde_json::json!({
        "bpm": w.bpm, "energy": w.energy, "timbral": w.timbral,
        "key": w.key, "genre": w.genre, "brightness": w.brightness, "rhythm": w.rhythm,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::scoring::{PoolWeights, PriorityWeights};

    fn approx_eq(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    // --- renormalize_transition ---

    #[test]
    fn renormalize_transition_normal() {
        let mut w = PriorityWeights {
            key: 2.0,
            bpm: 1.0,
            energy: 1.0,
            genre: 0.0,
            brightness: 0.0,
            rhythm: 0.0,
        };
        renormalize_transition(&mut w).unwrap();
        let sum = w.key + w.bpm + w.energy + w.genre + w.brightness + w.rhythm;
        assert!(approx_eq(sum, 1.0), "sum should be 1.0, got {sum}");
        assert!(approx_eq(w.key, 0.5));
        assert!(approx_eq(w.bpm, 0.25));
        assert!(approx_eq(w.energy, 0.25));
    }

    #[test]
    fn renormalize_transition_all_zero() {
        let mut w = PriorityWeights {
            key: 0.0,
            bpm: 0.0,
            energy: 0.0,
            genre: 0.0,
            brightness: 0.0,
            rhythm: 0.0,
        };
        let err = renormalize_transition(&mut w).unwrap_err();
        assert!(err.contains("All weights are zero"));
    }

    #[test]
    fn renormalize_transition_single_nonzero() {
        let mut w = PriorityWeights {
            key: 0.0,
            bpm: 0.0,
            energy: 3.0,
            genre: 0.0,
            brightness: 0.0,
            rhythm: 0.0,
        };
        renormalize_transition(&mut w).unwrap();
        assert!(approx_eq(w.energy, 1.0));
        assert!(approx_eq(w.key, 0.0));
    }

    #[test]
    fn renormalize_transition_negative_rejected() {
        let mut w = PriorityWeights {
            key: -1.0,
            bpm: 1.0,
            energy: 1.0,
            genre: 0.0,
            brightness: 0.0,
            rhythm: 0.0,
        };
        let err = renormalize_transition(&mut w).unwrap_err();
        assert!(err.contains("Negative weight"));
    }

    // --- renormalize_pool ---

    #[test]
    fn renormalize_pool_normal() {
        let mut w = PoolWeights {
            bpm: 1.0,
            energy: 1.0,
            timbral: 1.0,
            key: 1.0,
            genre: 1.0,
            brightness: 1.0,
            rhythm: 1.0,
        };
        renormalize_pool(&mut w).unwrap();
        let sum = w.bpm + w.energy + w.timbral + w.key + w.genre + w.brightness + w.rhythm;
        assert!(approx_eq(sum, 1.0), "sum should be 1.0, got {sum}");
        // All equal, each should be 1/7
        assert!(approx_eq(w.bpm, 1.0 / 7.0));
    }

    #[test]
    fn renormalize_pool_all_zero() {
        let mut w = PoolWeights {
            bpm: 0.0,
            energy: 0.0,
            timbral: 0.0,
            key: 0.0,
            genre: 0.0,
            brightness: 0.0,
            rhythm: 0.0,
        };
        let err = renormalize_pool(&mut w).unwrap_err();
        assert!(err.contains("All weights are zero"));
    }

    #[test]
    fn renormalize_pool_single_nonzero() {
        let mut w = PoolWeights {
            bpm: 0.0,
            energy: 0.0,
            timbral: 5.0,
            key: 0.0,
            genre: 0.0,
            brightness: 0.0,
            rhythm: 0.0,
        };
        renormalize_pool(&mut w).unwrap();
        assert!(approx_eq(w.timbral, 1.0));
        assert!(approx_eq(w.bpm, 0.0));
    }

    #[test]
    fn renormalize_pool_negative_rejected() {
        let mut w = PoolWeights {
            bpm: 1.0,
            energy: -0.5,
            timbral: 1.0,
            key: 1.0,
            genre: 1.0,
            brightness: 1.0,
            rhythm: 1.0,
        };
        let err = renormalize_pool(&mut w).unwrap_err();
        assert!(err.contains("Negative weight"));
    }

    // --- apply_transition_overrides ---

    #[test]
    fn apply_transition_overrides_one_axis() {
        let mut w = PriorityWeights {
            key: 0.3,
            bpm: 0.2,
            energy: 0.2,
            genre: 0.1,
            brightness: 0.1,
            rhythm: 0.1,
        };
        let overrides = TransitionWeightInput {
            key: Some(0.9),
            bpm: None,
            energy: None,
            genre: None,
            brightness: None,
            rhythm: None,
        };
        apply_transition_overrides(&mut w, &overrides);
        assert!(approx_eq(w.key, 0.9));
        // Other fields unchanged
        assert!(approx_eq(w.bpm, 0.2));
        assert!(approx_eq(w.energy, 0.2));
    }

    #[test]
    fn apply_transition_overrides_all_axes() {
        let mut w = PriorityWeights {
            key: 0.1,
            bpm: 0.1,
            energy: 0.1,
            genre: 0.1,
            brightness: 0.1,
            rhythm: 0.1,
        };
        let overrides = TransitionWeightInput {
            key: Some(1.0),
            bpm: Some(2.0),
            energy: Some(3.0),
            genre: Some(4.0),
            brightness: Some(5.0),
            rhythm: Some(6.0),
        };
        apply_transition_overrides(&mut w, &overrides);
        assert!(approx_eq(w.key, 1.0));
        assert!(approx_eq(w.bpm, 2.0));
        assert!(approx_eq(w.energy, 3.0));
        assert!(approx_eq(w.genre, 4.0));
        assert!(approx_eq(w.brightness, 5.0));
        assert!(approx_eq(w.rhythm, 6.0));
    }

    // --- apply_pool_overrides ---

    #[test]
    fn apply_pool_overrides_one_axis() {
        let mut w = PoolWeights {
            bpm: 0.2,
            energy: 0.2,
            timbral: 0.1,
            key: 0.2,
            genre: 0.1,
            brightness: 0.1,
            rhythm: 0.1,
        };
        let overrides = PoolWeightInput {
            bpm: None,
            energy: Some(0.8),
            timbral: None,
            key: None,
            genre: None,
            brightness: None,
            rhythm: None,
        };
        apply_pool_overrides(&mut w, &overrides);
        assert!(approx_eq(w.energy, 0.8));
        // Other fields unchanged
        assert!(approx_eq(w.bpm, 0.2));
        assert!(approx_eq(w.timbral, 0.1));
    }

    #[test]
    fn apply_pool_overrides_all_axes() {
        let mut w = PoolWeights {
            bpm: 0.0,
            energy: 0.0,
            timbral: 0.0,
            key: 0.0,
            genre: 0.0,
            brightness: 0.0,
            rhythm: 0.0,
        };
        let overrides = PoolWeightInput {
            bpm: Some(1.0),
            energy: Some(2.0),
            timbral: Some(3.0),
            key: Some(4.0),
            genre: Some(5.0),
            brightness: Some(6.0),
            rhythm: Some(7.0),
        };
        apply_pool_overrides(&mut w, &overrides);
        assert!(approx_eq(w.bpm, 1.0));
        assert!(approx_eq(w.energy, 2.0));
        assert!(approx_eq(w.timbral, 3.0));
        assert!(approx_eq(w.key, 4.0));
        assert!(approx_eq(w.genre, 5.0));
        assert!(approx_eq(w.brightness, 6.0));
        assert!(approx_eq(w.rhythm, 7.0));
    }
}
