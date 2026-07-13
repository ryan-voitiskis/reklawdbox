//! MCP planning parameter and JSON presentation mappings.
//!
//! Scoring policy lives in domain planning; stateful profile and timbral
//! orchestration lives in application planning.

use crate::domain::planning as domain;
use crate::mcp::planning::{PoolPreset, SequencingPriority};

use crate::domain::planning::{
    PoolAxisScores, PoolWeights, PriorityWeights, TransitionScores, round_to_3_decimals,
};

pub(in crate::mcp) fn priority_weights(priority: SequencingPriority) -> PriorityWeights {
    let priority = match priority {
        SequencingPriority::Balanced => domain::SequencingPriority::Balanced,
        SequencingPriority::Harmonic => domain::SequencingPriority::Harmonic,
        SequencingPriority::Energy => domain::SequencingPriority::Energy,
        SequencingPriority::Genre => domain::SequencingPriority::Genre,
    };
    domain::priority_weights(priority)
}

pub(in crate::mcp) fn pool_weights(preset: PoolPreset) -> PoolWeights {
    let preset = match preset {
        PoolPreset::Balanced => domain::PoolPreset::Balanced,
        PoolPreset::Timbral => domain::PoolPreset::Timbral,
    };
    domain::pool_weights(preset)
}

pub(in crate::mcp) trait TransitionScoresPresentation {
    fn to_json(&self) -> serde_json::Value;
}

impl TransitionScoresPresentation for TransitionScores {
    fn to_json(&self) -> serde_json::Value {
        let mut json = serde_json::json!({
            "key": { "value": round_to_3_decimals(self.key.value), "label": self.key.label },
            "bpm": { "value": round_to_3_decimals(self.bpm.value), "label": self.bpm.label },
            "energy": { "value": round_to_3_decimals(self.energy.value), "label": self.energy.label },
            "genre": { "value": round_to_3_decimals(self.genre.value), "label": self.genre.label },
            "brightness": { "value": round_to_3_decimals(self.brightness.value), "label": self.brightness.label },
            "rhythm": { "value": round_to_3_decimals(self.rhythm.value), "label": self.rhythm.label },
            "composite": round_to_3_decimals(self.composite),
        });
        if !self.adjustments.is_empty() {
            json["adjustments"] = serde_json::json!(
                self.adjustments
                    .iter()
                    .map(|adjustment| serde_json::json!({
                        "kind": adjustment.kind,
                        "delta": round_to_3_decimals(adjustment.delta),
                        "composite_without": round_to_3_decimals(adjustment.composite_without),
                        "reason": adjustment.reason,
                    }))
                    .collect::<Vec<_>>()
            );
        }
        json
    }
}

pub(in crate::mcp) trait PoolAxisScoresPresentation {
    fn to_json(&self) -> serde_json::Value;
}

impl PoolAxisScoresPresentation for PoolAxisScores {
    fn to_json(&self) -> serde_json::Value {
        let mut json = serde_json::json!({
            "key": { "value": round_to_3_decimals(self.key.value), "label": self.key.label },
            "bpm": { "value": round_to_3_decimals(self.bpm.value), "label": self.bpm.label },
            "energy": { "value": round_to_3_decimals(self.energy.value), "label": self.energy.label },
            "genre": { "value": round_to_3_decimals(self.genre.value), "label": self.genre.label },
            "brightness": { "value": round_to_3_decimals(self.brightness.value), "label": self.brightness.label },
            "rhythm": { "value": round_to_3_decimals(self.rhythm.value), "label": self.rhythm.label },
            "composite": round_to_3_decimals(self.composite),
        });
        if let Some(ref timbral) = self.timbral {
            json["timbral"] = serde_json::json!({
                "value": round_to_3_decimals(timbral.value),
                "label": timbral.label,
            });
        }
        json
    }
}
