use schemars::JsonSchema;
use serde::Deserialize;

use crate::mcp::library::SearchFilterParams;

#[derive(Debug, Deserialize)]
#[serde(tag = "operation")]
pub(in crate::mcp) enum AuditOperation {
    #[serde(rename = "scan")]
    Scan {
        #[serde(rename = "scope")]
        path_prefix: String,
        revalidate: Option<bool>,
        skip_issue_types: Option<Vec<String>>,
    },

    #[serde(rename = "query_issues")]
    QueryIssues {
        #[serde(rename = "scope")]
        path_prefix: String,
        status: Option<String>,
        issue_type: Option<String>,
        limit: Option<u32>,
        offset: Option<u32>,
    },

    #[serde(rename = "resolve_issues")]
    ResolveIssues {
        issue_ids: Vec<i64>,
        resolution: String,
        note: Option<String>,
    },

    #[serde(rename = "get_summary")]
    GetSummary {
        #[serde(rename = "scope")]
        path_prefix: String,
    },
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub(in crate::mcp) struct ScanBrokenLinksParams {
    #[schemars(description = "Scope to tracks whose file path starts with this prefix")]
    pub path_prefix: Option<String>,
    #[schemars(
        description = "Attempt case-insensitive filename matching for relocations (default true)"
    )]
    pub suggest_relocations: Option<bool>,
    #[schemars(description = "Max broken links to report (default 200)")]
    pub limit: Option<u32>,
    #[schemars(description = "Offset for pagination")]
    pub offset: Option<u32>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub(in crate::mcp) struct ScanOrphanFilesParams {
    #[schemars(description = "Directory to scan (default: content roots from library)")]
    pub path_prefix: Option<String>,
    #[schemars(description = "Max orphan files to report (default 200)")]
    pub limit: Option<u32>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub(in crate::mcp) struct ScanPlaylistCoverageParams {
    #[serde(flatten)]
    pub filters: SearchFilterParams,
    #[schemars(description = "Max uncovered tracks to return (default 200)")]
    pub limit: Option<u32>,
    #[schemars(description = "Offset for pagination")]
    pub offset: Option<u32>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "snake_case")]
pub(in crate::mcp) enum DuplicateDetectionLevel {
    /// Byte-identical file matching via SHA-256 hash
    Exact,
    /// Match by artist + title (case-insensitive)
    #[default]
    Metadata,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub(in crate::mcp) struct ScanDuplicatesParams {
    #[schemars(description = "Detection level: 'metadata' (default) or 'exact' (SHA-256 hash)")]
    pub detection_level: Option<DuplicateDetectionLevel>,
    #[schemars(description = "Scope to tracks whose file path starts with this prefix")]
    pub path_prefix: Option<String>,
    #[schemars(description = "Max duplicate groups to report (default 50)")]
    pub limit: Option<u32>,
    #[schemars(description = "Offset into the stable ordered duplicate-group list (default 0)")]
    pub offset: Option<u32>,
}

impl schemars::JsonSchema for AuditOperation {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed("AuditOperation")
    }

    fn json_schema(_gen: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "object",
            "required": ["operation"],
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["scan", "query_issues", "resolve_issues", "get_summary"],
                    "description": "The audit operation to perform"
                },
                "scope": {
                    "type": "string",
                    "description": "Directory path prefix (required for scan, query_issues, get_summary)"
                },
                "revalidate": {
                    "type": "boolean",
                    "description": "Re-read all files including unchanged (default: false). Only for scan."
                },
                "skip_issue_types": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Issue types to exclude from detection (e.g. [\"GENRE_SET\"]). Only for scan."
                },
                "status": {
                    "type": "string",
                    "description": "Filter by status: open | resolved | accepted | deferred. Only for query_issues."
                },
                "issue_type": {
                    "type": "string",
                    "description": "Filter by issue type (e.g. WAV_TAG3_MISSING). Only for query_issues."
                },
                "limit": {
                    "type": "integer",
                    "description": "Max results (default: 100). Only for query_issues."
                },
                "offset": {
                    "type": "integer",
                    "description": "Offset for pagination (default: 0). Only for query_issues."
                },
                "issue_ids": {
                    "type": "array",
                    "items": { "type": "integer" },
                    "description": "Issue IDs to resolve. Required for resolve_issues."
                },
                "resolution": {
                    "type": "string",
                    "description": "Resolution: accepted_as_is | wont_fix | deferred. Required for resolve_issues."
                },
                "note": {
                    "type": "string",
                    "description": "Optional user comment. Only for resolve_issues."
                }
            }
        })
    }
}
