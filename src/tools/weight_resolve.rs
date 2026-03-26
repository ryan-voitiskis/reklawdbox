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
