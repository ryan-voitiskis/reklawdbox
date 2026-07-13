use rusqlite::Connection;

use crate::domain::planning::{PoolWeights, PriorityWeights};
use crate::mcp::planning::{
    PoolPreset, PoolWeightInput, PoolWeightSpec, SequencingPriority, TransitionWeightInput,
    TransitionWeightSpec,
};

pub(in crate::mcp) fn resolve_transition_weights(
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
    crate::application::planning::resolve_transition_named(name, store)
}

macro_rules! impl_weight_ops {
    (
        apply_overrides: $apply_fn:ident,
        input_to_weights: $itow_fn:ident,
        renormalize: $renorm_fn:ident,
        to_json: $json_fn:ident,
        weights_ty: $weights_ty:ident,
        input_ty: $input_ty:ident,
        base_expr: $base_expr:expr,
        fields: [$($field:ident),+]
    ) => {
        pub(in crate::mcp) fn $apply_fn(w: &mut $weights_ty, overrides: &$input_ty) {
            $(if let Some(v) = overrides.$field { w.$field = v; })+
        }

        pub(in crate::mcp) fn $itow_fn(input: &$input_ty) -> $weights_ty {
            let base = $base_expr;
            $weights_ty { $($field: input.$field.unwrap_or(base.$field),)+ }
        }

        pub(in crate::mcp) fn $renorm_fn(w: &mut $weights_ty) -> Result<(), String> {
            crate::domain::planning::$renorm_fn(w)
        }

        pub(in crate::mcp) fn $json_fn(w: &$weights_ty) -> serde_json::Value {
            serde_json::json!({ $(stringify!($field): w.$field),+ })
        }
    };
}

impl_weight_ops! {
    apply_overrides: apply_transition_overrides,
    input_to_weights: transition_input_to_weights,
    renormalize: renormalize_transition,
    to_json: transition_weights_to_json,
    weights_ty: PriorityWeights,
    input_ty: TransitionWeightInput,
    base_expr: super::scoring::priority_weights(SequencingPriority::Balanced),
    fields: [key, bpm, energy, genre, brightness, rhythm]
}

impl_weight_ops! {
    apply_overrides: apply_pool_overrides,
    input_to_weights: pool_input_to_weights,
    renormalize: renormalize_pool,
    to_json: pool_weights_to_json,
    weights_ty: PoolWeights,
    input_ty: PoolWeightInput,
    base_expr: super::scoring::pool_weights(PoolPreset::Balanced),
    fields: [bpm, energy, timbral, key, genre, brightness, rhythm]
}

pub(in crate::mcp) fn resolve_pool_weights(
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
    crate::application::planning::resolve_pool_named(name, store)
}

#[cfg(test)]
mod tests {
    use super::*;

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
