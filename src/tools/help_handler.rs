use rmcp::ErrorData as McpError;
use rmcp::model::{CallToolResult, Content};
use schemars::JsonSchema;
use serde::Deserialize;

use super::mcp_internal_error;

const SOP_BATCH_IMPORT: &str = include_str!("../../site/src/partials/sops/batch-import.mdx");
const SOP_GENRE_CLASSIFICATION: &str =
    include_str!("../../site/src/partials/sops/genre-classification.mdx");
const SOP_GENRE_AUDIT: &str = include_str!("../../site/src/partials/sops/genre-audit.mdx");
const SOP_SET_BUILDING: &str = include_str!("../../site/src/partials/sops/set-building.mdx");
const SOP_COLLECTION_AUDIT: &str =
    include_str!("../../site/src/partials/sops/collection-audit.mdx");
const SOP_LABEL_BACKFILL: &str = include_str!("../../site/src/partials/sops/label-backfill.mdx");

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct HelpParams {
    #[schemars(
        description = "Optional topic filter: 'genre', 'genre audit', 'set', 'audit', 'import', 'label'. Omit for the full menu."
    )]
    pub topic: Option<String>,
}

struct Workflow {
    name: &'static str,
    keywords: &'static [&'static str],
    summary: &'static str,
    key_tools: &'static [&'static str],
    sop: &'static str,
}

const WORKFLOWS: &[Workflow] = &[
    Workflow {
        name: "Batch Import",
        keywords: &["import", "batch", "download", "purchase"],
        summary: "Prepare newly acquired music for Rekordbox import: tag, rename, embed cover art.",
        key_tools: &[
            "read_file_tags",
            "write_file_tags",
            "lookup_discogs",
            "lookup_beatport",
            "embed_cover_art",
            "extract_cover_art",
        ],
        sop: SOP_BATCH_IMPORT,
    },
    Workflow {
        name: "Genre Classification",
        keywords: &["genre", "classify", "classification"],
        summary: "Classify genres using Discogs, Beatport, and audio evidence.",
        key_tools: &[
            "cache_coverage",
            "classify_tracks",
            "suggest_normalizations",
            "update_tracks",
        ],
        sop: SOP_GENRE_CLASSIFICATION,
    },
    Workflow {
        name: "Set Building",
        keywords: &["set", "mix", "sequence", "transition", "playlist"],
        summary: "Build transition-scored DJ set sequences.",
        key_tools: &[
            "search_tracks",
            "resolve_tracks_data",
            "build_set",
            "score_transition",
        ],
        sop: SOP_SET_BUILDING,
    },
    Workflow {
        name: "Collection Audit",
        keywords: &["audit", "fix", "naming", "tags", "convention"],
        summary: "Detect and fix naming, tagging, and convention violations.",
        key_tools: &["audit_state", "read_file_tags", "write_file_tags"],
        sop: SOP_COLLECTION_AUDIT,
    },
    Workflow {
        name: "Genre Audit",
        keywords: &["genre audit", "verify genre", "genre conflict"],
        summary: "Verify existing genre tags against enrichment and audio evidence.",
        key_tools: &["cache_coverage", "audit_genres", "update_tracks"],
        sop: SOP_GENRE_AUDIT,
    },
    Workflow {
        name: "Label Backfill",
        keywords: &["label", "labels", "backfill", "publisher"],
        summary: "Auto-fill empty labels from Discogs enrichment, then resolve conflicts.",
        key_tools: &["cache_coverage", "backfill_labels", "update_tracks"],
        sop: SOP_LABEL_BACKFILL,
    },
];

pub(super) fn handle_help(params: HelpParams) -> Result<CallToolResult, McpError> {
    let result = if let Some(ref topic) = params.topic {
        let topic_lower = topic.to_lowercase();
        let matched = WORKFLOWS
            .iter()
            .filter_map(|w| {
                w.keywords
                    .iter()
                    .filter(|k| topic_lower.contains(*k))
                    .map(|k| k.len())
                    .max()
                    .map(|len| (w, len))
            })
            .max_by_key(|(_, len)| *len)
            .map(|(w, _)| w);

        match matched {
            Some(w) => serde_json::json!({
                "workflow": w.name,
                "key_tools": w.key_tools,
                "sop": w.sop,
            }),
            None => serde_json::json!({
                "error": format!("No workflow matching '{topic}'. Try: import, genre, genre audit, set, audit, label."),
                "workflows": WORKFLOWS.iter().map(|w| w.name).collect::<Vec<_>>(),
            }),
        }
    } else {
        serde_json::json!({
            "workflows": WORKFLOWS.iter().map(|w| {
                serde_json::json!({
                    "name": w.name,
                    "summary": w.summary,
                    "key_tools": w.key_tools,
                })
            }).collect::<Vec<_>>(),
            "recommended_order": concat!(
                "For disorganized libraries, workflow order matters — each step's quality depends on the previous:\n",
                "1. Collection Audit — fix artist/title naming (enrichment matches on these)\n",
                "2. Genre Classification — classify ungenred tracks with classify_tracks\n",
                "3. Genre Audit — verify existing genre tags with audit_genres\n",
                "4. Set Building — build DJ sets from well-tagged tracks\n",
                "Prerequisite: run `reklawdbox hydrate` from the CLI first to populate enrichment and analysis caches.\n",
                "Full guide: https://reklawdbox.com/workflows/library-cleanup/",
            ),
            "getting_started": "https://reklawdbox.com/getting-started/",
            "reference": "https://reklawdbox.com/reference/tools/",
            "tip": "Call help(topic='genre'|'import'|'set'|'audit'|'genre audit'|'label') for the full step-by-step SOP.",
        })
    };

    let json =
        serde_json::to_string_pretty(&result).map_err(|e| mcp_internal_error(format!("{e}")))?;
    Ok(CallToolResult::success(vec![Content::text(json)]))
}
