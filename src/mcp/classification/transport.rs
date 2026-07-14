use schemars::JsonSchema;
use serde::Deserialize;

use crate::mcp::library::SearchFilterParams;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ClassifyTracksParams {
    #[serde(flatten)]
    pub filters: SearchFilterParams,
    #[schemars(description = "Specific track IDs to classify (highest priority selector)")]
    pub track_ids: Option<Vec<String>>,
    #[schemars(description = "Classify tracks in this playlist")]
    pub playlist_id: Option<String>,
    #[schemars(description = "Max tracks to classify (default 50, max 200)")]
    pub max_tracks: Option<u32>,
    #[schemars(description = "Offset for pagination (skip first N tracks)")]
    pub offset: Option<u32>,
    #[schemars(
        description = "Genre overrides: remap a genre string before scoring. Example: [{\"from\": \"Melodic House & Techno\", \"to\": \"Deep Techno\"}]"
    )]
    pub genre_overrides: Option<Vec<GenreOverrideInput>>,
    #[schemars(
        description = "Response format: 'full' (default) returns evidence, candidates, flags, and review hints. 'compact' returns only track_id, artist, title, genre, confidence, action. 'summary' returns confidence distribution and genre-grouped counts. 'dispatch' returns low/insufficient tracks grouped by artist. Weak confirmations remain visible in review formats."
    )]
    pub format: Option<ClassifyFormat>,
    #[schemars(
        description = "Auto-stage real genre changes at these confidence levels after classification. Confirmations, genre-less results, and insufficient results are never staged. Example: [\"high\", \"medium\"]. Omit for classification only."
    )]
    pub auto_stage: Option<Vec<StageLevel>>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "snake_case")]
pub enum ClassifyFormat {
    #[default]
    Full,
    Compact,
    Summary,
    Dispatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "snake_case")]
pub enum StageLevel {
    High,
    Medium,
    Low,
    Insufficient,
}

impl StageLevel {
    pub fn matches_confidence(
        &self,
        conf: &crate::domain::classification::ClassificationConfidence,
    ) -> bool {
        use crate::domain::classification::ClassificationConfidence;
        matches!(
            (self, conf),
            (Self::High, ClassificationConfidence::High)
                | (Self::Medium, ClassificationConfidence::Medium)
                | (Self::Low, ClassificationConfidence::Low)
                | (Self::Insufficient, ClassificationConfidence::Insufficient)
        )
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(inline)]
pub struct CalibrateAudioProfilesParams {
    #[schemars(
        description = "Name of the playlist containing verified tracks (default: 'genre_verified')"
    )]
    pub playlist: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(inline)]
pub struct CalibrationCoverageParams {
    #[schemars(
        description = "Name of the playlist containing verified tracks (default: 'genre_verified')"
    )]
    pub playlist: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(inline)]
pub struct AuditGenresParams {
    #[serde(flatten)]
    pub filters: SearchFilterParams,
    #[schemars(description = "Specific track IDs to audit (highest priority selector)")]
    pub track_ids: Option<Vec<String>>,
    #[schemars(description = "Audit tracks in this playlist")]
    pub playlist_id: Option<String>,
    #[schemars(description = "Max tracks to audit (default 50, max 200)")]
    pub max_tracks: Option<u32>,
    #[schemars(description = "Offset for pagination (skip first N tracks)")]
    pub offset: Option<u32>,
    #[schemars(
        description = "Include every confirmed track (default false). Low/insufficient confirmations are always included because they require review."
    )]
    pub include_confirmed: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(inline)]
pub struct GenreOverrideInput {
    #[schemars(description = "Source genre string to match (case-insensitive)")]
    pub from: String,
    #[schemars(description = "Target canonical genre to use instead")]
    pub to: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::classification::ClassificationConfidence;

    #[test]
    fn stage_level_matches_only_corresponding_confidence() {
        let cases = [
            (StageLevel::High, ClassificationConfidence::High, true),
            (StageLevel::High, ClassificationConfidence::Medium, false),
            (StageLevel::High, ClassificationConfidence::Low, false),
            (
                StageLevel::High,
                ClassificationConfidence::Insufficient,
                false,
            ),
            (StageLevel::Medium, ClassificationConfidence::High, false),
            (StageLevel::Medium, ClassificationConfidence::Medium, true),
            (StageLevel::Medium, ClassificationConfidence::Low, false),
            (
                StageLevel::Medium,
                ClassificationConfidence::Insufficient,
                false,
            ),
            (StageLevel::Low, ClassificationConfidence::High, false),
            (StageLevel::Low, ClassificationConfidence::Medium, false),
            (StageLevel::Low, ClassificationConfidence::Low, true),
            (
                StageLevel::Low,
                ClassificationConfidence::Insufficient,
                false,
            ),
            (
                StageLevel::Insufficient,
                ClassificationConfidence::High,
                false,
            ),
            (
                StageLevel::Insufficient,
                ClassificationConfidence::Medium,
                false,
            ),
            (
                StageLevel::Insufficient,
                ClassificationConfidence::Low,
                false,
            ),
            (
                StageLevel::Insufficient,
                ClassificationConfidence::Insufficient,
                true,
            ),
        ];
        for (level, conf, expected) in cases {
            assert_eq!(
                level.matches_confidence(&conf),
                expected,
                "{level:?} vs {conf:?}"
            );
        }
    }
}
