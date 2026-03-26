use rmcp::ErrorData as McpError;
use rmcp::model::{CallToolResult, Content};

use super::*;

pub(super) fn handle_save_weight_preset(
    server: &ReklawdboxServer,
    params: SaveWeightPresetParams,
) -> Result<CallToolResult, McpError> {
    let scorer_type_str = match params.scorer_type {
        ScorerType::Pool => "pool",
        ScorerType::Transition => "transition",
    };

    let is_built_in = matches!(
        (scorer_type_str, params.name.as_str()),
        ("transition", "balanced" | "harmonic" | "energy" | "genre")
            | ("pool", "balanced" | "timbral")
    );
    if is_built_in {
        return Err(McpError::invalid_params(
            format!(
                "Cannot overwrite built-in preset '{}'. Choose a different name.",
                params.name
            ),
            None,
        ));
    }

    let normalized_json = match params.scorer_type {
        ScorerType::Transition => {
            let input: TransitionWeightInput = serde_json::from_value(params.weights)
                .map_err(|e| {
                    McpError::invalid_params(
                        format!("Invalid transition weights: {e}. Expected: {{key, bpm, energy, genre, brightness, rhythm}}"),
                        None,
                    )
                })?;
            let mut w = weight_resolve::transition_input_to_weights(&input);
            weight_resolve::renormalize_transition(&mut w)
                .map_err(|e| McpError::invalid_params(e, None))?;
            weight_resolve::transition_weights_to_json(&w)
        }
        ScorerType::Pool => {
            let input: PoolWeightInput = serde_json::from_value(params.weights).map_err(|e| {
                McpError::invalid_params(
                    format!("Invalid pool weights: {e}. Expected: {{bpm, energy, timbral, key, genre, brightness, rhythm}}"),
                    None,
                )
            })?;
            let mut w = weight_resolve::pool_input_to_weights(&input);
            weight_resolve::renormalize_pool(&mut w)
                .map_err(|e| McpError::invalid_params(e, None))?;
            weight_resolve::pool_weights_to_json(&w)
        }
    };

    let json_str =
        serde_json::to_string(&normalized_json).map_err(|e| mcp_internal_error(format!("{e}")))?;

    let store = server.cache_store_conn()?;
    crate::store::save_weight_preset(&store, &params.name, scorer_type_str, &json_str)
        .map_err(|e| mcp_internal_error(format!("Failed to save preset: {e}")))?;

    let result = serde_json::json!({
        "saved": params.name,
        "scorer_type": scorer_type_str,
        "weights": normalized_json,
    });

    let json =
        serde_json::to_string_pretty(&result).map_err(|e| mcp_internal_error(format!("{e}")))?;
    Ok(CallToolResult::success(vec![Content::text(json)]))
}

pub(super) fn handle_list_weight_presets(
    server: &ReklawdboxServer,
    params: ListWeightPresetsParams,
) -> Result<CallToolResult, McpError> {
    let scorer_type_str = params.scorer_type.map(|st| match st {
        ScorerType::Pool => "pool",
        ScorerType::Transition => "transition",
    });

    let store = server.cache_store_conn()?;
    let saved = crate::store::list_weight_presets(&store, scorer_type_str)
        .map_err(|e| mcp_internal_error(format!("Failed to list presets: {e}")))?;

    let mut presets: Vec<serde_json::Value> = Vec::new();

    if scorer_type_str.is_none() || scorer_type_str == Some("transition") {
        for (name, priority) in [
            ("balanced", SequencingPriority::Balanced),
            ("harmonic", SequencingPriority::Harmonic),
            ("energy", SequencingPriority::Energy),
            ("genre", SequencingPriority::Genre),
        ] {
            let w = scoring::priority_weights(priority);
            presets.push(serde_json::json!({
                "name": name,
                "scorer_type": "transition",
                "built_in": true,
                "weights": weight_resolve::transition_weights_to_json(&w),
            }));
        }
    }

    if scorer_type_str.is_none() || scorer_type_str == Some("pool") {
        for (name, preset) in [
            ("balanced", PoolPreset::Balanced),
            ("timbral", PoolPreset::Timbral),
        ] {
            let w = scoring::pool_weights(preset);
            presets.push(serde_json::json!({
                "name": name,
                "scorer_type": "pool",
                "built_in": true,
                "weights": weight_resolve::pool_weights_to_json(&w),
            }));
        }
    }

    for entry in saved {
        let weights: serde_json::Value =
            serde_json::from_str(&entry.weights_json).unwrap_or(serde_json::Value::Null);
        presets.push(serde_json::json!({
            "name": entry.name,
            "scorer_type": entry.scorer_type,
            "built_in": false,
            "weights": weights,
        }));
    }

    let result = serde_json::json!({ "presets": presets });
    let json =
        serde_json::to_string_pretty(&result).map_err(|e| mcp_internal_error(format!("{e}")))?;
    Ok(CallToolResult::success(vec![Content::text(json)]))
}

pub(super) fn handle_delete_weight_preset(
    server: &ReklawdboxServer,
    params: DeleteWeightPresetParams,
) -> Result<CallToolResult, McpError> {
    let scorer_type_str = match params.scorer_type {
        ScorerType::Pool => "pool",
        ScorerType::Transition => "transition",
    };

    let built_in = matches!(
        (scorer_type_str, params.name.as_str()),
        ("transition", "balanced" | "harmonic" | "energy" | "genre")
            | ("pool", "balanced" | "timbral")
    );
    if built_in {
        return Err(McpError::invalid_params(
            format!("Cannot delete built-in preset '{}'", params.name),
            None,
        ));
    }

    let store = server.cache_store_conn()?;
    let deleted = crate::store::delete_weight_preset(&store, &params.name, scorer_type_str)
        .map_err(|e| mcp_internal_error(format!("Failed to delete preset: {e}")))?;

    let result = serde_json::json!({
        "deleted": deleted,
        "name": params.name,
        "scorer_type": scorer_type_str,
    });
    let json =
        serde_json::to_string_pretty(&result).map_err(|e| mcp_internal_error(format!("{e}")))?;
    Ok(CallToolResult::success(vec![Content::text(json)]))
}
