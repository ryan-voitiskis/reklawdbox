use rmcp::ErrorData as McpError;
use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;

use super::ok_json;

const SOP_BATCH_IMPORT: &str = include_str!("../../site/src/partials/sops/batch-import.mdx");
const SOP_GENRE_CLASSIFICATION: &str =
    include_str!("../../site/src/partials/sops/genre-classification.mdx");
const SOP_GENRE_AUDIT: &str = include_str!("../../site/src/partials/sops/genre-audit.mdx");
const SOP_SET_BUILDING: &str = include_str!("../../site/src/partials/sops/set-building.mdx");
const SOP_COLLECTION_AUDIT: &str =
    include_str!("../../site/src/partials/sops/collection-audit.mdx");
const SOP_METADATA_BACKFILL: &str =
    include_str!("../../site/src/partials/sops/metadata-backfill.mdx");
const SOP_LIBRARY_HEALTH: &str = include_str!("../../site/src/partials/sops/library-health.mdx");
const SOP_POOL_BUILDING: &str = include_str!("../../site/src/partials/sops/pool-building.mdx");
const SOP_CHAPTER_SET_PLANNING: &str =
    include_str!("../../site/src/partials/sops/chapter-set-planning.mdx");
const SOP_XML_PLAYLIST_IMPORT_STEPS: &str =
    include_str!("../../site/src/partials/sops/xml-playlist-import-steps.mdx");

const XML_PLAYLIST_IMPORT_COMPONENT_IMPORT: &str =
    "import XmlPlaylistImportSteps from './xml-playlist-import-steps.mdx';";
const XML_PLAYLIST_IMPORT_COMPONENT_TAG: &str = "<XmlPlaylistImportSteps />";

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct HelpParams {
    #[schemars(
        description = "Optional topic filter: 'genre', 'genre audit', 'set', 'pool', 'chapter', 'audit', 'import', 'metadata', 'label', 'year', 'album', 'health'. Omit for the full menu."
    )]
    pub topic: Option<String>,
}

struct Workflow {
    name: &'static str,
    keywords: &'static [&'static str],
    summary: &'static str,
    key_tools: &'static [&'static str],
    sop: &'static str,
    expand_playlist_import: bool,
}

const WORKFLOWS: &[Workflow] = &[
    Workflow {
        name: "Collection Audit",
        keywords: &["audit", "fix", "naming", "tags", "convention"],
        summary: "Detect and fix naming, tagging, and convention violations.",
        key_tools: &["audit_state", "read_file_tags", "write_file_tags"],
        sop: SOP_COLLECTION_AUDIT,
        expand_playlist_import: false,
    },
    Workflow {
        name: "Metadata Backfill",
        keywords: &[
            "label",
            "labels",
            "album",
            "albums",
            "backfill",
            "publisher",
            "year",
            "years",
            "metadata",
        ],
        summary: "Auto-fill empty labels, years, and albums from enrichment and file metadata.",
        key_tools: &[
            "cache_coverage",
            "backfill_labels",
            "backfill_years",
            "backfill_albums",
            "update_tracks",
        ],
        sop: SOP_METADATA_BACKFILL,
        expand_playlist_import: false,
    },
    Workflow {
        name: "Genre Classification",
        keywords: &["genre", "classify", "classification"],
        summary: "Classify genres using Discogs, Beatport, and audio evidence.",
        key_tools: &[
            "cache_coverage",
            "calibration_coverage",
            "classify_tracks",
            "suggest_normalizations",
            "update_tracks",
        ],
        sop: SOP_GENRE_CLASSIFICATION,
        expand_playlist_import: false,
    },
    Workflow {
        name: "Genre Audit",
        keywords: &["genre audit", "verify genre", "genre conflict"],
        summary: "Verify existing genre tags against enrichment and audio evidence.",
        key_tools: &[
            "cache_coverage",
            "calibration_coverage",
            "audit_genres",
            "update_tracks",
        ],
        sop: SOP_GENRE_AUDIT,
        expand_playlist_import: false,
    },
    Workflow {
        name: "Set Building",
        keywords: &["set", "mix", "sequence", "transition", "playlist"],
        summary: "Build transition-scored DJ set sequences.",
        key_tools: &[
            "get_sessions",
            "get_play_stats",
            "search_tracks",
            "resolve_tracks_data",
            "build_set",
            "score_transition",
        ],
        sop: SOP_SET_BUILDING,
        expand_playlist_import: true,
    },
    Workflow {
        name: "Pool Building",
        keywords: &["pool", "crate", "expand", "discover"],
        summary: "Build compatible track pools for live improvisation using symmetric pool scoring.",
        key_tools: &[
            "expand_pool",
            "describe_pool",
            "score_pool_compatibility",
            "write_xml",
        ],
        sop: SOP_POOL_BUILDING,
        expand_playlist_import: true,
    },
    Workflow {
        name: "Chapter Set Planning",
        keywords: &["chapter", "set plan", "bridge", "gig prep", "night plan"],
        summary: "Plan a full set from locked chapters with bridge tracks between them.",
        key_tools: &[
            "describe_pool",
            "expand_pool",
            "score_pool_compatibility",
            "build_set",
            "score_transition",
            "write_xml",
        ],
        sop: SOP_CHAPTER_SET_PLANNING,
        expand_playlist_import: true,
    },
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
        expand_playlist_import: false,
    },
    Workflow {
        name: "Library Health",
        keywords: &[
            "health",
            "broken",
            "orphan",
            "duplicate",
            "coverage",
            "missing",
            "dead",
        ],
        summary: "Detect broken links, orphan files, duplicates, and playlist gaps.",
        key_tools: &[
            "scan_broken_links",
            "scan_orphan_files",
            "scan_playlist_coverage",
            "scan_duplicates",
        ],
        sop: SOP_LIBRARY_HEALTH,
        expand_playlist_import: false,
    },
];

fn expand_playlist_import_steps(sop: &str) -> Result<String, McpError> {
    let import_count = sop
        .lines()
        .filter(|line| *line == XML_PLAYLIST_IMPORT_COMPONENT_IMPORT)
        .count();
    let tag_count = sop.matches(XML_PLAYLIST_IMPORT_COMPONENT_TAG).count();

    if import_count != 1 || tag_count != 1 {
        return Err(McpError::internal_error(
            format!(
                "playlist import SOP composition requires exactly one component import and tag; found {import_count} import(s) and {tag_count} tag(s)"
            ),
            None,
        ));
    }

    let import_line = format!("{XML_PLAYLIST_IMPORT_COMPONENT_IMPORT}\n");
    let without_import = sop.replacen(&import_line, "", 1);
    Ok(without_import.replacen(
        XML_PLAYLIST_IMPORT_COMPONENT_TAG,
        SOP_XML_PLAYLIST_IMPORT_STEPS,
        1,
    ))
}

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
            Some(w) => {
                let sop = if w.expand_playlist_import {
                    expand_playlist_import_steps(w.sop)?
                } else {
                    w.sop.to_owned()
                };
                serde_json::json!({
                    "workflow": w.name,
                    "key_tools": w.key_tools,
                    "sop": sop,
                })
            }
            None => serde_json::json!({
                "error": format!("No workflow matching '{topic}'. Try: import, genre, genre audit, set, pool, chapter, audit, metadata, label, year, album, health."),
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
                "2. Metadata Backfill — fill labels and years from Discogs enrichment\n",
                "3. Genre Classification — classify ungenred tracks with classify_tracks\n",
                "4. Genre Audit — verify existing genre tags with audit_genres\n",
                "5. Set Building — build DJ sets from well-tagged tracks\n",
                "6. Pool Building — discover compatible track pools for live improvisation\n",
                "7. Chapter Set Planning — plan a full night from locked chapters (requires pools)\n",
                "Prerequisite: run `reklawdbox hydrate` from the CLI first to populate enrichment and analysis caches.\n",
                "Full guide: https://reklawdbox.com/workflows/library-cleanup/",
            ),
            "getting_started": "https://reklawdbox.com/getting-started/",
            "reference": "https://reklawdbox.com/mcp-tools/",
            "tip": "Call help(topic='genre'|'import'|'set'|'pool'|'chapter'|'audit'|'genre audit'|'metadata'|'label'|'year'|'health') for the full step-by-step SOP.",
        })
    };

    ok_json(&result)
}
