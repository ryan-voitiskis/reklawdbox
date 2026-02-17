use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use rmcp::handler::server::tool::ToolRouter;
use rmcp::model::{CallToolResult, Content, ServerCapabilities, ServerInfo};
use rmcp::{ErrorData as McpError, ServerHandler, tool, tool_handler, tool_router};
use rusqlite::Connection;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::beatport;
use crate::changes::ChangeManager;
use crate::corpus;
use crate::db;
use crate::discogs;
use crate::genre;
use crate::types::TrackChange;
use crate::xml;

/// Inner shared state (not Clone).
struct ServerState {
    db: OnceLock<Result<Mutex<Connection>, String>>,
    db_path: Option<String>,
    changes: ChangeManager,
    http: reqwest::Client,
}

/// MCP server for Rekordbox library management.
#[derive(Clone)]
pub struct CrateDigServer {
    state: Arc<ServerState>,
    tool_router: ToolRouter<Self>,
}

impl CrateDigServer {
    fn conn(&self) -> Result<std::sync::MutexGuard<'_, Connection>, McpError> {
        let result = self.state.db.get_or_init(|| {
            let path = match self.state.db_path.clone().or_else(db::resolve_db_path) {
                Some(p) => p,
                None => {
                    return Err(
                        "Rekordbox database not found. Set REKORDBOX_DB_PATH env var.".into(),
                    );
                }
            };
            match db::open(&path) {
                Ok(conn) => Ok(Mutex::new(conn)),
                Err(e) => Err(format!("Failed to open Rekordbox database: {e}")),
            }
        });
        match result {
            Ok(mutex) => mutex
                .lock()
                .map_err(|_| McpError::internal_error("Database lock poisoned", None)),
            Err(msg) => Err(McpError::internal_error(msg.clone(), None)),
        }
    }
}

fn err(msg: String) -> McpError {
    McpError::internal_error(msg, None)
}

struct CorpusConsultation {
    consulted_documents: Vec<String>,
    manifest_status: String,
    warning: Option<String>,
}

#[derive(Clone, Copy)]
struct CorpusQuerySpec {
    topic: Option<&'static str>,
    mode: Option<&'static str>,
    doc_type: Option<&'static str>,
    search_text: Option<&'static str>,
    limit: usize,
}

fn unique_paths(paths: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for path in paths {
        if seen.insert(path.clone()) {
            out.push(path);
        }
    }
    out
}

fn fallback_corpus_consultation(
    fallback_paths: &[&str],
    manifest_status: &str,
    warning: Option<String>,
) -> CorpusConsultation {
    CorpusConsultation {
        consulted_documents: unique_paths(fallback_paths.iter().map(|p| (*p).to_string())),
        manifest_status: manifest_status.to_string(),
        warning,
    }
}

fn consult_manifest_first_docs(
    query_specs: &[CorpusQuerySpec],
    fallback_paths: &[&str],
) -> CorpusConsultation {
    match corpus::rekordbox_index() {
        Ok(index) => {
            let mut paths = Vec::new();
            for query_spec in query_specs {
                let query = corpus::CorpusQuery {
                    topic: query_spec.topic,
                    mode: query_spec.mode,
                    doc_type: query_spec.doc_type,
                    search_text: query_spec.search_text,
                    limit: Some(query_spec.limit),
                };
                paths.extend(index.consulted_paths(query));
            }

            let paths = unique_paths(paths);
            if paths.is_empty() {
                return fallback_corpus_consultation(
                    fallback_paths,
                    "empty",
                    Some(
                        "Corpus retrieval returned no matching documents; used fallback references."
                            .to_string(),
                    ),
                );
            }

            CorpusConsultation {
                consulted_documents: paths,
                manifest_status: "ok".to_string(),
                warning: None,
            }
        }
        Err(e) => fallback_corpus_consultation(
            fallback_paths,
            "unavailable",
            Some(format!(
                "Corpus retrieval failed; used fallback references: {e}"
            )),
        ),
    }
}

fn consult_xml_workflow_docs() -> CorpusConsultation {
    consult_manifest_first_docs(
        &[
            CorpusQuerySpec {
                topic: Some("xml"),
                mode: Some("export"),
                doc_type: Some("reference"),
                search_text: Some("xml import export"),
                limit: 3,
            },
            CorpusQuerySpec {
                topic: Some("xml"),
                mode: Some("common"),
                doc_type: Some("guide"),
                search_text: Some("xml format"),
                limit: 3,
            },
            CorpusQuerySpec {
                topic: Some("xml"),
                mode: Some("common"),
                doc_type: Some("reference"),
                search_text: Some("developer integration"),
                limit: 3,
            },
            CorpusQuerySpec {
                topic: Some("library"),
                mode: Some("common"),
                doc_type: Some("faq"),
                search_text: Some("xml"),
                limit: 2,
            },
        ],
        &[
            "docs/rekordbox/reference/xml-import-export.md",
            "docs/rekordbox/guides/xml-format-spec.md",
            "docs/rekordbox/reference/developer-integration.md",
            "docs/rekordbox/manual/31-preferences.md",
            "docs/rekordbox/faq/library-and-collection.md",
        ],
    )
}

fn consult_genre_workflow_docs() -> CorpusConsultation {
    consult_manifest_first_docs(
        &[
            CorpusQuerySpec {
                topic: Some("genre"),
                mode: Some("common"),
                doc_type: Some("manual"),
                search_text: Some("genre"),
                limit: 3,
            },
            CorpusQuerySpec {
                topic: Some("metadata"),
                mode: Some("common"),
                doc_type: Some("reference"),
                search_text: Some("genre metadata"),
                limit: 3,
            },
            CorpusQuerySpec {
                topic: Some("library"),
                mode: Some("common"),
                doc_type: Some("faq"),
                search_text: Some("genre"),
                limit: 3,
            },
            CorpusQuerySpec {
                topic: Some("collection"),
                mode: Some("common"),
                doc_type: Some("manual"),
                search_text: Some("search genre"),
                limit: 3,
            },
        ],
        &[
            "docs/rekordbox/manual/06-searching.md",
            "docs/rekordbox/faq/library-and-collection.md",
            "docs/rekordbox/reference/glossary.md",
            "docs/rekordbox/reference/developer-integration.md",
        ],
    )
}

fn attach_corpus_provenance(result: &mut serde_json::Value, consultation: CorpusConsultation) {
    result["consulted_documents"] = serde_json::json!(consultation.consulted_documents);
    result["manifest_status"] = serde_json::json!(consultation.manifest_status);
    if let Some(warning) = consultation.warning {
        result["corpus_warning"] = serde_json::json!(warning);
    }
}

// --- Tool parameter types ---

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchTracksParams {
    #[schemars(description = "Search query matching title or artist")]
    pub query: Option<String>,
    #[schemars(description = "Filter by artist name (partial match)")]
    pub artist: Option<String>,
    #[schemars(description = "Filter by genre name (partial match)")]
    pub genre: Option<String>,
    #[schemars(description = "Minimum star rating (1-5)")]
    pub rating_min: Option<u8>,
    #[schemars(description = "Minimum BPM")]
    pub bpm_min: Option<f64>,
    #[schemars(description = "Maximum BPM")]
    pub bpm_max: Option<f64>,
    #[schemars(description = "Filter by musical key (e.g. 'Am', 'Cm')")]
    pub key: Option<String>,
    #[schemars(description = "Filter by playlist ID")]
    pub playlist: Option<String>,
    #[schemars(description = "Filter by whether track has a genre set")]
    pub has_genre: Option<bool>,
    #[schemars(description = "Include Rekordbox factory samples (default false)")]
    pub include_samples: Option<bool>,
    #[schemars(description = "Max results (default 50, max 200)")]
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetTrackParams {
    #[schemars(description = "Track ID")]
    pub track_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetPlaylistTracksParams {
    #[schemars(description = "Playlist ID")]
    pub playlist_id: String,
    #[schemars(description = "Max results (default 200)")]
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateTracksParams {
    #[schemars(description = "Array of track changes to stage")]
    pub changes: Vec<TrackChangeInput>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TrackChangeInput {
    #[schemars(description = "Track ID")]
    pub track_id: String,
    #[schemars(description = "New genre")]
    pub genre: Option<String>,
    #[schemars(description = "New comments")]
    pub comments: Option<String>,
    #[schemars(description = "New star rating (1-5)")]
    pub rating: Option<u8>,
    #[schemars(description = "New color name")]
    pub color: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WriteXmlParams {
    #[schemars(
        description = "Output file path (default: ./rekordbox-exports/crate-dig-{timestamp}.xml)"
    )]
    pub output_path: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ClearChangesParams {
    #[schemars(description = "Track IDs to clear (if empty, clears all)")]
    pub track_ids: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SuggestNormalizationsParams {
    #[schemars(description = "Only show genres with at least this many tracks (default 1)")]
    pub min_count: Option<i32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LookupDiscogsParams {
    #[schemars(description = "Artist name")]
    pub artist: String,
    #[schemars(description = "Track title")]
    pub title: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LookupBeatportParams {
    #[schemars(description = "Artist name")]
    pub artist: String,
    #[schemars(description = "Track title")]
    pub title: String,
}

// --- Tool implementations ---

use rmcp::handler::server::wrapper::Parameters;

#[tool_router]
impl CrateDigServer {
    pub fn new(db_path: Option<String>) -> Self {
        let http = reqwest::Client::builder()
            .user_agent("CrateGuide/2.0")
            .build()
            .expect("failed to build HTTP client");
        Self {
            state: Arc::new(ServerState {
                db: OnceLock::new(),
                db_path,
                changes: ChangeManager::new(),
                http,
            }),
            tool_router: Self::tool_router(),
        }
    }

    #[tool(description = "Search and filter tracks in the Rekordbox library")]
    async fn search_tracks(
        &self,
        params: Parameters<SearchTracksParams>,
    ) -> Result<CallToolResult, McpError> {
        let conn = self.conn()?;
        let search = db::SearchParams {
            query: params.0.query,
            artist: params.0.artist,
            genre: params.0.genre,
            rating_min: params.0.rating_min,
            bpm_min: params.0.bpm_min,
            bpm_max: params.0.bpm_max,
            key: params.0.key,
            playlist: params.0.playlist,
            has_genre: params.0.has_genre,
            exclude_samples: !params.0.include_samples.unwrap_or(false),
            limit: params.0.limit,
        };
        let tracks =
            db::search_tracks(&conn, &search).map_err(|e| err(format!("DB error: {e}")))?;
        let json = serde_json::to_string_pretty(&tracks).map_err(|e| err(format!("{e}")))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Get full details for a specific track by ID")]
    async fn get_track(
        &self,
        params: Parameters<GetTrackParams>,
    ) -> Result<CallToolResult, McpError> {
        let conn = self.conn()?;
        let track =
            db::get_track(&conn, &params.0.track_id).map_err(|e| err(format!("DB error: {e}")))?;
        match track {
            Some(t) => {
                let json = serde_json::to_string_pretty(&t).map_err(|e| err(format!("{e}")))?;
                Ok(CallToolResult::success(vec![Content::text(json)]))
            }
            None => Ok(CallToolResult::success(vec![Content::text(format!(
                "Track '{}' not found",
                params.0.track_id
            ))])),
        }
    }

    #[tool(description = "List all playlists with track counts")]
    async fn get_playlists(&self) -> Result<CallToolResult, McpError> {
        let conn = self.conn()?;
        let playlists = db::get_playlists(&conn).map_err(|e| err(format!("DB error: {e}")))?;
        let json = serde_json::to_string_pretty(&playlists).map_err(|e| err(format!("{e}")))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "List tracks in a specific playlist")]
    async fn get_playlist_tracks(
        &self,
        params: Parameters<GetPlaylistTracksParams>,
    ) -> Result<CallToolResult, McpError> {
        let conn = self.conn()?;
        let tracks = db::get_playlist_tracks(&conn, &params.0.playlist_id, params.0.limit)
            .map_err(|e| err(format!("DB error: {e}")))?;
        let json = serde_json::to_string_pretty(&tracks).map_err(|e| err(format!("{e}")))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Get library summary: track count, genre distribution, stats")]
    async fn read_library(&self) -> Result<CallToolResult, McpError> {
        let conn = self.conn()?;
        let stats = db::get_library_stats(&conn).map_err(|e| err(format!("DB error: {e}")))?;
        let json = serde_json::to_string_pretty(&stats).map_err(|e| err(format!("{e}")))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Get the configured genre taxonomy")]
    async fn get_genre_taxonomy(&self) -> Result<CallToolResult, McpError> {
        let genres = genre::get_taxonomy();
        let aliases: std::collections::HashMap<String, String> =
            genre::get_alias_map().into_iter().collect();
        let mut result = serde_json::json!({
            "genres": genres,
            "aliases": aliases,
            "description": "Flat genre taxonomy. Not a closed list — arbitrary genres are accepted. This list provides consistency suggestions. Aliases map non-canonical genre names to their canonical forms."
        });
        attach_corpus_provenance(&mut result, consult_genre_workflow_docs());
        let json = serde_json::to_string_pretty(&result).map_err(|e| err(format!("{e}")))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        description = "Stage changes to track metadata (genre, comments, rating, color). Changes are held in memory until write_xml is called."
    )]
    async fn update_tracks(
        &self,
        params: Parameters<UpdateTracksParams>,
    ) -> Result<CallToolResult, McpError> {
        // Validate ratings before staging
        for c in &params.0.changes {
            if let Some(r) = c.rating {
                if r == 0 || r > 5 {
                    return Err(McpError::invalid_params(
                        format!("rating must be 1-5, got {r}"),
                        None,
                    ));
                }
            }
        }

        // Collect genre warnings for non-taxonomy genres
        let mut warnings: Vec<String> = Vec::new();
        for c in &params.0.changes {
            if let Some(ref g) = c.genre {
                if !genre::is_known_genre(g) {
                    warnings.push(format!("'{}' is not in the genre taxonomy", g));
                }
            }
        }

        let changes: Vec<TrackChange> = params
            .0
            .changes
            .into_iter()
            .map(|c| TrackChange {
                track_id: c.track_id,
                genre: c.genre,
                comments: c.comments,
                rating: c.rating,
                color: c.color,
            })
            .collect();

        let (staged, total) = self.state.changes.stage(changes);
        let mut result = serde_json::json!({
            "staged": staged,
            "total_pending": total,
        });
        if !warnings.is_empty() {
            result["warnings"] = serde_json::json!(warnings);
        }
        attach_corpus_provenance(&mut result, consult_genre_workflow_docs());
        let json = serde_json::to_string_pretty(&result).map_err(|e| err(format!("{e}")))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        description = "Analyze all genres in the library and suggest normalizations. Returns alias (known mapping), unknown (needs manual decision), and canonical (already correct) sections."
    )]
    async fn suggest_normalizations(
        &self,
        params: Parameters<SuggestNormalizationsParams>,
    ) -> Result<CallToolResult, McpError> {
        let conn = self.conn()?;
        let min_count = params.0.min_count.unwrap_or(1);

        let stats = db::get_library_stats(&conn).map_err(|e| err(format!("DB error: {e}")))?;

        let mut alias_suggestions = Vec::new();
        let mut unknown_items = Vec::new();
        let mut canonical_items = Vec::new();

        for gc in &stats.genres {
            if gc.name == "(none)" || gc.name.is_empty() {
                continue;
            }
            if gc.count < min_count {
                continue;
            }

            if let Some(canonical) = genre::normalize_genre(&gc.name) {
                // Known alias — fetch tracks for this genre
                let tracks = db::get_tracks_by_exact_genre(&conn, &gc.name, true)
                    .map_err(|e| err(format!("DB error: {e}")))?;
                for t in tracks {
                    alias_suggestions.push(crate::types::NormalizationSuggestion {
                        track_id: t.id,
                        title: t.title,
                        artist: t.artist,
                        current_genre: gc.name.clone(),
                        suggested_genre: Some(canonical.to_string()),
                        confidence: "alias".to_string(),
                    });
                }
            } else if genre::is_known_genre(&gc.name) {
                canonical_items.push(serde_json::json!({
                    "genre": gc.name,
                    "count": gc.count,
                }));
            } else {
                // Unknown genre — not in taxonomy or alias map
                let tracks = db::get_tracks_by_exact_genre(&conn, &gc.name, true)
                    .map_err(|e| err(format!("DB error: {e}")))?;
                for t in tracks {
                    unknown_items.push(crate::types::NormalizationSuggestion {
                        track_id: t.id,
                        title: t.title,
                        artist: t.artist,
                        current_genre: gc.name.clone(),
                        suggested_genre: None,
                        confidence: "unknown".to_string(),
                    });
                }
            }
        }

        let mut result = serde_json::json!({
            "alias": alias_suggestions,
            "unknown": unknown_items,
            "canonical": canonical_items,
            "summary": {
                "alias_tracks": alias_suggestions.len(),
                "unknown_tracks": unknown_items.len(),
                "canonical_genres": canonical_items.len(),
            }
        });
        attach_corpus_provenance(&mut result, consult_genre_workflow_docs());
        let json = serde_json::to_string_pretty(&result).map_err(|e| err(format!("{e}")))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Preview all staged changes, showing what will differ from current state")]
    async fn preview_changes(&self) -> Result<CallToolResult, McpError> {
        let ids = self.state.changes.pending_ids();
        if ids.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(
                "No changes staged.",
            )]));
        }

        let conn = self.conn()?;
        let current_tracks =
            db::get_tracks_by_ids(&conn, &ids).map_err(|e| err(format!("DB error: {e}")))?;

        let diffs = self.state.changes.preview(&current_tracks);
        if diffs.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(
                "Changes staged but no fields actually differ from current values.",
            )]));
        }

        let json = serde_json::to_string_pretty(&diffs).map_err(|e| err(format!("{e}")))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        description = "Write staged changes to a Rekordbox-compatible XML file. Runs backup first."
    )]
    async fn write_xml(
        &self,
        params: Parameters<WriteXmlParams>,
    ) -> Result<CallToolResult, McpError> {
        let ids = self.state.changes.pending_ids();
        if ids.is_empty() {
            let mut result = serde_json::json!({
                "message": "No changes to write.",
                "track_count": 0,
                "changes_applied": 0,
            });
            attach_corpus_provenance(&mut result, consult_xml_workflow_docs());
            let json = serde_json::to_string_pretty(&result).map_err(|e| err(format!("{e}")))?;
            return Ok(CallToolResult::success(vec![Content::text(json)]));
        }

        // Run backup before writing
        let backup_script = std::path::Path::new("backup.sh");
        if backup_script.exists() {
            eprintln!("[crate-dig] Running pre-op backup...");
            let output = std::process::Command::new("bash")
                .arg("backup.sh")
                .arg("--pre-op")
                .output();
            match output {
                Ok(o) if o.status.success() => eprintln!("[crate-dig] Backup completed."),
                Ok(o) => {
                    let stderr_out = String::from_utf8_lossy(&o.stderr);
                    eprintln!("[crate-dig] Backup warning: {stderr_out}");
                }
                Err(e) => eprintln!("[crate-dig] Backup skipped: {e}"),
            }
        }

        // Fetch current tracks and apply changes
        let conn = self.conn()?;
        let current_tracks =
            db::get_tracks_by_ids(&conn, &ids).map_err(|e| err(format!("DB error: {e}")))?;
        let modified_tracks = self.state.changes.apply_changes(&current_tracks);

        // Determine output path
        let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let output_path = params.0.output_path.map(PathBuf::from).unwrap_or_else(|| {
            PathBuf::from(format!("rekordbox-exports/crate-dig-{timestamp}.xml"))
        });

        xml::write_xml(&modified_tracks, &output_path)
            .map_err(|e| err(format!("Write error: {e}")))?;

        let track_count = modified_tracks.len();
        // Clear staged changes after successful write
        self.state.changes.clear(None);

        let mut result = serde_json::json!({
            "path": output_path.to_string_lossy(),
            "track_count": track_count,
            "changes_applied": track_count,
        });
        attach_corpus_provenance(&mut result, consult_xml_workflow_docs());
        let json = serde_json::to_string_pretty(&result).map_err(|e| err(format!("{e}")))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Clear staged changes for specific tracks or all")]
    async fn clear_changes(
        &self,
        params: Parameters<ClearChangesParams>,
    ) -> Result<CallToolResult, McpError> {
        let (cleared, remaining) = self.state.changes.clear(params.0.track_ids);
        let result = serde_json::json!({
            "cleared": cleared,
            "remaining": remaining,
        });
        let json = serde_json::to_string_pretty(&result).map_err(|e| err(format!("{e}")))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        description = "Look up a track on Discogs for genre/style enrichment. Returns release info with genres and styles, or null if not found."
    )]
    async fn lookup_discogs(
        &self,
        params: Parameters<LookupDiscogsParams>,
    ) -> Result<CallToolResult, McpError> {
        let result = discogs::lookup(&self.state.http, &params.0.artist, &params.0.title)
            .await
            .map_err(|e| err(format!("Discogs error: {e}")))?;
        let json = serde_json::to_string_pretty(&result).map_err(|e| err(format!("{e}")))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        description = "Look up a track on Beatport for genre/BPM/key enrichment. Returns track info or null if not found."
    )]
    async fn lookup_beatport(
        &self,
        params: Parameters<LookupBeatportParams>,
    ) -> Result<CallToolResult, McpError> {
        let result = beatport::lookup(&self.state.http, &params.0.artist, &params.0.title)
            .await
            .map_err(|e| err(format!("Beatport error: {e}")))?;
        let json = serde_json::to_string_pretty(&result).map_err(|e| err(format!("{e}")))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }
}

#[tool_handler]
impl ServerHandler for CrateDigServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "Rekordbox library management server. Search tracks, manage genres, \
                 stage metadata changes, and export to XML for reimport."
                    .into(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extract_json(result: &CallToolResult) -> serde_json::Value {
        let text = result
            .content
            .first()
            .and_then(|content| content.as_text())
            .map(|text| text.text.as_str())
            .expect("tool result should include text content");

        serde_json::from_str(text).expect("tool text content should be valid JSON")
    }

    fn assert_has_provenance(payload: &serde_json::Value) {
        let docs = payload
            .get("consulted_documents")
            .and_then(serde_json::Value::as_array)
            .expect("consulted_documents should be an array");
        assert!(
            !docs.is_empty(),
            "consulted_documents should include at least one document"
        );
        assert!(
            docs.iter().all(serde_json::Value::is_string),
            "consulted_documents should contain document paths"
        );
        let manifest_status = payload
            .get("manifest_status")
            .and_then(serde_json::Value::as_str)
            .expect("manifest_status should be a string");
        assert!(
            !manifest_status.is_empty(),
            "manifest_status should not be empty"
        );
    }

    #[tokio::test]
    async fn write_xml_no_change_path_includes_provenance() {
        let server = CrateDigServer::new(None);

        let result = server
            .write_xml(Parameters(WriteXmlParams { output_path: None }))
            .await
            .expect("write_xml should succeed when no changes are staged");

        let payload = extract_json(&result);
        assert_eq!(
            payload
                .get("message")
                .and_then(serde_json::Value::as_str)
                .expect("message should be present"),
            "No changes to write."
        );
        assert_has_provenance(&payload);
    }

    #[tokio::test]
    async fn update_tracks_includes_provenance() {
        let server = CrateDigServer::new(None);
        let known_genre = genre::get_taxonomy()
            .into_iter()
            .next()
            .unwrap_or_else(|| "House".to_string());

        let result = server
            .update_tracks(Parameters(UpdateTracksParams {
                changes: vec![TrackChangeInput {
                    track_id: "test-track-1".to_string(),
                    genre: Some(known_genre),
                    comments: Some("staged by test".to_string()),
                    rating: Some(4),
                    color: None,
                }],
            }))
            .await
            .expect("update_tracks should succeed");

        let payload = extract_json(&result);
        assert_eq!(
            payload
                .get("staged")
                .and_then(serde_json::Value::as_u64)
                .expect("staged should be present"),
            1
        );
        assert_eq!(
            payload
                .get("total_pending")
                .and_then(serde_json::Value::as_u64)
                .expect("total_pending should be present"),
            1
        );
        assert_has_provenance(&payload);
    }
}
