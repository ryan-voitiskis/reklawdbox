//! Vocabulary shared by enrichment workflows and transports.

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// External metadata provider available to enrichment workflows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "lowercase")]
pub(crate) enum EnrichmentProvider {
    Discogs,
    Beatport,
    Bandcamp,
}

impl EnrichmentProvider {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Discogs => "discogs",
            Self::Beatport => "beatport",
            Self::Bandcamp => "bandcamp",
        }
    }
}

impl fmt::Display for EnrichmentProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A stage selected by the public `hydrate` CLI command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HydrationStage {
    Lookup(EnrichmentProvider),
    Analysis,
}

/// Ordered hydration stages parsed from the CLI's comma-separated value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HydrationStages(Vec<HydrationStage>);

impl HydrationStages {
    pub(crate) fn parse_csv(value: &str) -> Result<Self, String> {
        let mut stages = Vec::new();
        for part in value.split(',') {
            let stage = match part.trim().to_ascii_lowercase().as_str() {
                "discogs" => HydrationStage::Lookup(EnrichmentProvider::Discogs),
                "beatport" => HydrationStage::Lookup(EnrichmentProvider::Beatport),
                "analysis" => HydrationStage::Analysis,
                other => return Err(format!("unknown provider: {other}")),
            };
            stages.push(stage);
        }
        if stages.is_empty() {
            return Err("no providers specified".into());
        }
        Ok(Self(stages))
    }

    pub(crate) fn contains(&self, stage: HydrationStage) -> bool {
        self.0.contains(&stage)
    }
}

/// Outcome of consulting a local enrichment cache before a provider lookup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CacheLookupOutcome<T> {
    Hit(T),
    Miss,
}

/// Provider lookup outcome before transport-specific rendering.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ProviderLookupOutcome<T> {
    Match(T),
    NoMatch,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enrichment_provider_preserves_public_json_spellings() {
        assert_eq!(
            serde_json::to_string(&EnrichmentProvider::Discogs).unwrap(),
            "\"discogs\""
        );
        assert_eq!(
            serde_json::to_string(&EnrichmentProvider::Beatport).unwrap(),
            "\"beatport\""
        );
        assert_eq!(
            serde_json::to_string(&EnrichmentProvider::Bandcamp).unwrap(),
            "\"bandcamp\""
        );
        assert_eq!(
            serde_json::from_str::<EnrichmentProvider>("\"discogs\"").unwrap(),
            EnrichmentProvider::Discogs
        );
    }

    #[test]
    fn hydration_stages_preserve_cli_spellings_and_order() {
        let stages = HydrationStages::parse_csv("discogs, beatport,analysis").unwrap();

        assert_eq!(
            stages,
            HydrationStages(vec![
                HydrationStage::Lookup(EnrichmentProvider::Discogs),
                HydrationStage::Lookup(EnrichmentProvider::Beatport),
                HydrationStage::Analysis,
            ])
        );
        assert!(stages.contains(HydrationStage::Lookup(EnrichmentProvider::Discogs)));
    }

    #[test]
    fn hydration_stages_preserve_unknown_provider_error() {
        assert_eq!(
            HydrationStages::parse_csv("bandcamp").unwrap_err(),
            "unknown provider: bandcamp"
        );
        assert_eq!(
            HydrationStages::parse_csv("").unwrap_err(),
            "unknown provider: "
        );
    }
}
