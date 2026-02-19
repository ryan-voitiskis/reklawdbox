use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use rmcp::handler::server::tool::ToolRouter;
use rmcp::model::{CallToolResult, Content, ServerCapabilities, ServerInfo};
use rmcp::{ErrorData as McpError, ServerHandler, tool, tool_handler, tool_router};
use rusqlite::Connection;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::audio;
use crate::beatport;
use crate::changes::ChangeManager;
use crate::corpus;
use crate::db;
use crate::discogs;
use crate::genre;
use crate::store;
use crate::types::TrackChange;
use crate::xml;

/// Inner shared state (not Clone).
struct ServerState {
    db: OnceLock<Result<Mutex<Connection>, String>>,
    internal_db: OnceLock<Result<Mutex<Connection>, String>>,
    essentia_python: OnceLock<Option<String>>,
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

    fn internal_conn(&self) -> Result<std::sync::MutexGuard<'_, Connection>, McpError> {
        let result = self.state.internal_db.get_or_init(|| {
            let path = std::env::var("CRATE_DIG_STORE_PATH")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|_| store::default_path());
            let path_str = path.to_string_lossy().to_string();
            match store::open(&path_str) {
                Ok(conn) => Ok(Mutex::new(conn)),
                Err(e) => Err(format!("Failed to open internal store: {e}")),
            }
        });
        match result {
            Ok(mutex) => mutex
                .lock()
                .map_err(|_| McpError::internal_error("Internal store lock poisoned", None)),
            Err(msg) => Err(McpError::internal_error(msg.clone(), None)),
        }
    }

    fn essentia_python_path(&self) -> Option<String> {
        self.state
            .essentia_python
            .get_or_init(probe_essentia_python_path)
            .clone()
    }
}

fn err(msg: String) -> McpError {
    McpError::internal_error(msg, None)
}

const ESSENTIA_PYTHON_ENV_VAR: &str = "CRATE_DIG_ESSENTIA_PYTHON";
const ESSENTIA_IMPORT_CHECK_SCRIPT: &str = "import essentia; print(essentia.__version__)";

fn validate_essentia_python(path: &str) -> bool {
    std::process::Command::new(path)
        .args(["-c", ESSENTIA_IMPORT_CHECK_SCRIPT])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn probe_essentia_python_from_sources(
    env_override: Option<&str>,
    default_candidate: Option<PathBuf>,
) -> Option<String> {
    let mut candidates: Vec<String> = Vec::new();
    if let Some(path) = env_override.map(str::trim).filter(|path| !path.is_empty()) {
        candidates.push(path.to_string());
    }
    if let Some(path) = default_candidate {
        let path = path.to_string_lossy().to_string();
        if !path.is_empty() && !candidates.iter().any(|candidate| candidate == &path) {
            candidates.push(path);
        }
    }

    candidates
        .into_iter()
        .find(|candidate| validate_essentia_python(candidate))
}

fn probe_essentia_python_path() -> Option<String> {
    let env_override = std::env::var(ESSENTIA_PYTHON_ENV_VAR).ok();
    let default_candidate =
        dirs::home_dir().map(|home| home.join(".local/share/crate-dig/essentia-venv/bin/python"));
    probe_essentia_python_from_sources(env_override.as_deref(), default_candidate)
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
    #[schemars(description = "Bypass cache and fetch fresh data (default false)")]
    pub force_refresh: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LookupBeatportParams {
    #[schemars(description = "Artist name")]
    pub artist: String,
    #[schemars(description = "Track title")]
    pub title: String,
    #[schemars(description = "Bypass cache and fetch fresh data (default false)")]
    pub force_refresh: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct EnrichTracksParams {
    #[schemars(description = "Specific track IDs to enrich (highest priority selector)")]
    pub track_ids: Option<Vec<String>>,
    #[schemars(description = "Enrich tracks in this playlist")]
    pub playlist_id: Option<String>,
    #[schemars(description = "Search query matching title or artist")]
    pub query: Option<String>,
    #[schemars(description = "Filter by artist name (partial match)")]
    pub artist: Option<String>,
    #[schemars(description = "Filter by genre name (partial match)")]
    pub genre: Option<String>,
    #[schemars(description = "Filter by whether track has a genre set")]
    pub has_genre: Option<bool>,
    #[schemars(description = "Minimum BPM")]
    pub bpm_min: Option<f64>,
    #[schemars(description = "Maximum BPM")]
    pub bpm_max: Option<f64>,
    #[schemars(description = "Filter by musical key (e.g. 'Am', 'Cm')")]
    pub key: Option<String>,
    #[schemars(description = "Minimum star rating (1-5)")]
    pub rating_min: Option<u8>,
    #[schemars(description = "Max tracks to enrich (default 50)")]
    pub max_tracks: Option<u32>,
    #[schemars(description = "Providers to use: 'discogs', 'beatport' (default ['discogs'])")]
    pub providers: Option<Vec<String>>,
    #[schemars(description = "Skip tracks already in cache (default true)")]
    pub skip_cached: Option<bool>,
    #[schemars(description = "Bypass cache and fetch fresh data (default false)")]
    pub force_refresh: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AnalyzeTrackAudioParams {
    #[schemars(description = "Track ID to analyze")]
    pub track_id: String,
    #[schemars(description = "Skip if already cached (default true)")]
    pub skip_cached: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AnalyzeAudioBatchParams {
    #[schemars(description = "Specific track IDs to analyze (highest priority selector)")]
    pub track_ids: Option<Vec<String>>,
    #[schemars(description = "Analyze tracks in this playlist")]
    pub playlist_id: Option<String>,
    #[schemars(description = "Search query matching title or artist")]
    pub query: Option<String>,
    #[schemars(description = "Filter by artist name (partial match)")]
    pub artist: Option<String>,
    #[schemars(description = "Filter by genre name (partial match)")]
    pub genre: Option<String>,
    #[schemars(description = "Filter by whether track has a genre set")]
    pub has_genre: Option<bool>,
    #[schemars(description = "Minimum BPM")]
    pub bpm_min: Option<f64>,
    #[schemars(description = "Maximum BPM")]
    pub bpm_max: Option<f64>,
    #[schemars(description = "Filter by musical key")]
    pub key: Option<String>,
    #[schemars(description = "Minimum star rating (1-5)")]
    pub rating_min: Option<u8>,
    #[schemars(description = "Max tracks to analyze (default 20)")]
    pub max_tracks: Option<u32>,
    #[schemars(description = "Skip tracks already in cache (default true)")]
    pub skip_cached: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ResolveTrackDataParams {
    #[schemars(description = "Track ID to resolve")]
    pub track_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ResolveTracksDataParams {
    #[schemars(description = "Specific track IDs to resolve (highest priority selector)")]
    pub track_ids: Option<Vec<String>>,
    #[schemars(description = "Resolve tracks in this playlist")]
    pub playlist_id: Option<String>,
    #[schemars(description = "Search query matching title or artist")]
    pub query: Option<String>,
    #[schemars(description = "Filter by artist name (partial match)")]
    pub artist: Option<String>,
    #[schemars(description = "Filter by genre name (partial match)")]
    pub genre: Option<String>,
    #[schemars(description = "Filter by whether track has a genre set")]
    pub has_genre: Option<bool>,
    #[schemars(description = "Minimum BPM")]
    pub bpm_min: Option<f64>,
    #[schemars(description = "Maximum BPM")]
    pub bpm_max: Option<f64>,
    #[schemars(description = "Filter by musical key")]
    pub key: Option<String>,
    #[schemars(description = "Minimum star rating (1-5)")]
    pub rating_min: Option<u8>,
    #[schemars(description = "Max tracks to resolve (default 50)")]
    pub max_tracks: Option<u32>,
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
                internal_db: OnceLock::new(),
                essentia_python: OnceLock::new(),
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
        description = "Look up a track on Discogs for genre/style enrichment. Returns release info with genres and styles, or null if not found. Results are cached."
    )]
    async fn lookup_discogs(
        &self,
        params: Parameters<LookupDiscogsParams>,
    ) -> Result<CallToolResult, McpError> {
        let force_refresh = params.0.force_refresh.unwrap_or(false);
        let norm_artist = discogs::normalize(&params.0.artist);
        let norm_title = discogs::normalize(&params.0.title);

        // Check cache
        if !force_refresh {
            let store = self.internal_conn()?;
            if let Some(cached) =
                store::get_enrichment(&store, "discogs", &norm_artist, &norm_title)
                    .map_err(|e| err(format!("Cache read error: {e}")))?
            {
                let mut result = match &cached.response_json {
                    Some(json_str) => serde_json::from_str::<serde_json::Value>(json_str)
                        .unwrap_or(serde_json::Value::Null),
                    None => serde_json::Value::Null,
                };
                if let serde_json::Value::Object(ref mut map) = result {
                    map.insert("cache_hit".to_string(), serde_json::json!(true));
                    map.insert(
                        "cached_at".to_string(),
                        serde_json::json!(cached.created_at),
                    );
                } else {
                    result = serde_json::json!({
                        "result": null,
                        "cache_hit": true,
                        "cached_at": cached.created_at,
                    });
                }
                let json =
                    serde_json::to_string_pretty(&result).map_err(|e| err(format!("{e}")))?;
                return Ok(CallToolResult::success(vec![Content::text(json)]));
            }
        }

        // Cache miss — fetch from Discogs
        let result = discogs::lookup(&self.state.http, &params.0.artist, &params.0.title)
            .await
            .map_err(|e| err(format!("Discogs error: {e}")))?;

        // Store in cache
        let (match_quality, response_json) = match &result {
            Some(r) => {
                let quality = if r.fuzzy_match { "fuzzy" } else { "exact" };
                let json = serde_json::to_string(r).map_err(|e| err(format!("{e}")))?;
                (Some(quality), Some(json))
            }
            None => (Some("none"), None),
        };
        {
            let store = self.internal_conn()?;
            store::set_enrichment(
                &store,
                "discogs",
                &norm_artist,
                &norm_title,
                match_quality,
                response_json.as_deref(),
            )
            .map_err(|e| err(format!("Cache write error: {e}")))?;
        }

        let mut output = serde_json::to_value(&result).map_err(|e| err(format!("{e}")))?;
        if let serde_json::Value::Object(ref mut map) = output {
            map.insert("cache_hit".to_string(), serde_json::json!(false));
        }
        let json = serde_json::to_string_pretty(&output).map_err(|e| err(format!("{e}")))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        description = "Look up a track on Beatport for genre/BPM/key enrichment. Returns track info or null if not found. Results are cached."
    )]
    async fn lookup_beatport(
        &self,
        params: Parameters<LookupBeatportParams>,
    ) -> Result<CallToolResult, McpError> {
        let force_refresh = params.0.force_refresh.unwrap_or(false);
        let norm_artist = discogs::normalize(&params.0.artist);
        let norm_title = discogs::normalize(&params.0.title);

        // Check cache
        if !force_refresh {
            let store = self.internal_conn()?;
            if let Some(cached) =
                store::get_enrichment(&store, "beatport", &norm_artist, &norm_title)
                    .map_err(|e| err(format!("Cache read error: {e}")))?
            {
                let mut result = match &cached.response_json {
                    Some(json_str) => serde_json::from_str::<serde_json::Value>(json_str)
                        .unwrap_or(serde_json::Value::Null),
                    None => serde_json::Value::Null,
                };
                if let serde_json::Value::Object(ref mut map) = result {
                    map.insert("cache_hit".to_string(), serde_json::json!(true));
                    map.insert(
                        "cached_at".to_string(),
                        serde_json::json!(cached.created_at),
                    );
                } else {
                    result = serde_json::json!({
                        "result": null,
                        "cache_hit": true,
                        "cached_at": cached.created_at,
                    });
                }
                let json =
                    serde_json::to_string_pretty(&result).map_err(|e| err(format!("{e}")))?;
                return Ok(CallToolResult::success(vec![Content::text(json)]));
            }
        }

        // Cache miss — fetch from Beatport
        let result = beatport::lookup(&self.state.http, &params.0.artist, &params.0.title)
            .await
            .map_err(|e| err(format!("Beatport error: {e}")))?;

        // Store in cache
        let (match_quality, response_json) = match &result {
            Some(r) => {
                let json = serde_json::to_string(r).map_err(|e| err(format!("{e}")))?;
                (Some("exact"), Some(json))
            }
            None => (Some("none"), None),
        };
        {
            let store = self.internal_conn()?;
            store::set_enrichment(
                &store,
                "beatport",
                &norm_artist,
                &norm_title,
                match_quality,
                response_json.as_deref(),
            )
            .map_err(|e| err(format!("Cache write error: {e}")))?;
        }

        let mut output = serde_json::to_value(&result).map_err(|e| err(format!("{e}")))?;
        if let serde_json::Value::Object(ref mut map) = output {
            map.insert("cache_hit".to_string(), serde_json::json!(false));
        }
        let json = serde_json::to_string_pretty(&output).map_err(|e| err(format!("{e}")))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        description = "Batch enrich tracks via Discogs/Beatport. Select tracks by IDs, playlist, or search filters. Results are cached."
    )]
    async fn enrich_tracks(
        &self,
        params: Parameters<EnrichTracksParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let max_tracks = p.max_tracks.unwrap_or(50).min(200) as usize;
        let skip_cached = p.skip_cached.unwrap_or(true);
        let force_refresh = p.force_refresh.unwrap_or(false);
        let providers: Vec<String> = p.providers.unwrap_or_else(|| vec!["discogs".to_string()]);

        // Validate providers
        for prov in &providers {
            if prov != "discogs" && prov != "beatport" {
                return Err(McpError::invalid_params(
                    format!("unknown provider '{prov}', valid: discogs, beatport"),
                    None,
                ));
            }
        }

        // Resolve tracks using priority: track_ids > playlist_id > search filters
        let tracks = {
            let conn = self.conn()?;
            if let Some(ref ids) = p.track_ids {
                db::get_tracks_by_ids(&conn, ids).map_err(|e| err(format!("DB error: {e}")))?
            } else if let Some(ref playlist_id) = p.playlist_id {
                db::get_playlist_tracks(&conn, playlist_id, Some(max_tracks as u32))
                    .map_err(|e| err(format!("DB error: {e}")))?
            } else {
                let search = db::SearchParams {
                    query: p.query,
                    artist: p.artist,
                    genre: p.genre,
                    rating_min: p.rating_min,
                    bpm_min: p.bpm_min,
                    bpm_max: p.bpm_max,
                    key: p.key,
                    playlist: None,
                    has_genre: p.has_genre,
                    exclude_samples: true,
                    limit: Some(max_tracks as u32),
                };
                db::search_tracks(&conn, &search).map_err(|e| err(format!("DB error: {e}")))?
            }
        };

        let tracks: Vec<_> = tracks.into_iter().take(max_tracks).collect();
        let total = tracks.len();

        let mut enriched = 0usize;
        let mut cached = 0usize;
        let mut skipped = 0usize;
        let mut failed: Vec<serde_json::Value> = Vec::new();

        for track in &tracks {
            let norm_artist = discogs::normalize(&track.artist);
            let norm_title = discogs::normalize(&track.title);

            for provider in &providers {
                // Check cache first
                if skip_cached && !force_refresh {
                    let store = self.internal_conn()?;
                    if store::get_enrichment(&store, provider, &norm_artist, &norm_title)
                        .map_err(|e| err(format!("Cache read error: {e}")))?
                        .is_some()
                    {
                        cached += 1;
                        continue;
                    }
                }

                // Perform lookup and cache result per provider
                match provider.as_str() {
                    "discogs" => {
                        match discogs::lookup(&self.state.http, &track.artist, &track.title).await {
                            Ok(Some(r)) => {
                                let json_str =
                                    serde_json::to_string(&r).map_err(|e| err(format!("{e}")))?;
                                let quality = if r.fuzzy_match { "fuzzy" } else { "exact" };
                                let store = self.internal_conn()?;
                                store::set_enrichment(
                                    &store,
                                    provider,
                                    &norm_artist,
                                    &norm_title,
                                    Some(quality),
                                    Some(&json_str),
                                )
                                .map_err(|e| err(format!("Cache write error: {e}")))?;
                                enriched += 1;
                            }
                            Ok(None) => {
                                let store = self.internal_conn()?;
                                store::set_enrichment(
                                    &store,
                                    provider,
                                    &norm_artist,
                                    &norm_title,
                                    Some("none"),
                                    None,
                                )
                                .map_err(|e| err(format!("Cache write error: {e}")))?;
                                skipped += 1;
                            }
                            Err(e) => {
                                failed.push(serde_json::json!({
                                    "track_id": track.id,
                                    "artist": track.artist,
                                    "title": track.title,
                                    "provider": provider,
                                    "error": e,
                                }));
                            }
                        }
                    }
                    "beatport" => {
                        match beatport::lookup(&self.state.http, &track.artist, &track.title).await
                        {
                            Ok(Some(r)) => {
                                let json_str =
                                    serde_json::to_string(&r).map_err(|e| err(format!("{e}")))?;
                                let store = self.internal_conn()?;
                                store::set_enrichment(
                                    &store,
                                    provider,
                                    &norm_artist,
                                    &norm_title,
                                    Some("exact"),
                                    Some(&json_str),
                                )
                                .map_err(|e| err(format!("Cache write error: {e}")))?;
                                enriched += 1;
                            }
                            Ok(None) => {
                                let store = self.internal_conn()?;
                                store::set_enrichment(
                                    &store,
                                    provider,
                                    &norm_artist,
                                    &norm_title,
                                    Some("none"),
                                    None,
                                )
                                .map_err(|e| err(format!("Cache write error: {e}")))?;
                                skipped += 1;
                            }
                            Err(e) => {
                                failed.push(serde_json::json!({
                                    "track_id": track.id,
                                    "artist": track.artist,
                                    "title": track.title,
                                    "provider": provider,
                                    "error": e,
                                }));
                            }
                        }
                    }
                    _ => unreachable!(),
                }
            }
        }

        let result = serde_json::json!({
            "summary": {
                "total": total,
                "enriched": enriched,
                "cached": cached,
                "skipped": skipped,
                "failed": failed.len(),
            },
            "failures": failed,
        });
        let json = serde_json::to_string_pretty(&result).map_err(|e| err(format!("{e}")))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        description = "Analyze a single track's audio file with stratum-dsp and Essentia (when installed). Returns BPM, key, rhythm/loudness descriptors, and confidence scores. Results are cached."
    )]
    async fn analyze_track_audio(
        &self,
        params: Parameters<AnalyzeTrackAudioParams>,
    ) -> Result<CallToolResult, McpError> {
        let skip_cached = params.0.skip_cached.unwrap_or(true);

        let track = {
            let conn = self.conn()?;
            db::get_track(&conn, &params.0.track_id)
                .map_err(|e| err(format!("DB error: {e}")))?
                .ok_or_else(|| {
                    McpError::invalid_params(
                        format!("Track '{}' not found", params.0.track_id),
                        None,
                    )
                })?
        };

        let file_path = resolve_file_path(&track.file_path)?;
        let metadata = std::fs::metadata(&file_path)
            .map_err(|e| err(format!("Cannot stat file '{}': {e}", file_path)))?;
        let file_size = metadata.len() as i64;
        let file_mtime = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        let mut stratum_dsp: Option<serde_json::Value> = None;
        let mut stratum_cache_hit = false;

        if skip_cached {
            let store = self.internal_conn()?;
            if let Some(cached_entry) = store::get_audio_analysis(&store, &file_path, "stratum-dsp")
                .map_err(|e| err(format!("Cache read error: {e}")))?
            {
                if cached_entry.file_size == file_size && cached_entry.file_mtime == file_mtime {
                    stratum_dsp = Some(
                        serde_json::from_str(&cached_entry.features_json)
                            .map_err(|e| err(format!("Cache parse error: {e}")))?,
                    );
                    stratum_cache_hit = true;
                }
            }
        }

        if stratum_dsp.is_none() {
            let path_clone = file_path.clone();
            let (samples, sample_rate) =
                tokio::task::spawn_blocking(move || audio::decode_to_samples(&path_clone))
                    .await
                    .map_err(|e| err(format!("Decode task failed: {e}")))?
                    .map_err(|e| err(format!("Decode error: {e}")))?;

            let analysis =
                tokio::task::spawn_blocking(move || audio::analyze(&samples, sample_rate))
                    .await
                    .map_err(|e| err(format!("Analysis task failed: {e}")))?
                    .map_err(|e| err(format!("Analysis error: {e}")))?;

            let features_json =
                serde_json::to_string(&analysis).map_err(|e| err(format!("{e}")))?;
            {
                let store = self.internal_conn()?;
                store::set_audio_analysis(
                    &store,
                    &file_path,
                    "stratum-dsp",
                    file_size,
                    file_mtime,
                    &analysis.analyzer_version,
                    &features_json,
                )
                .map_err(|e| err(format!("Cache write error: {e}")))?;
            }

            stratum_dsp = Some(serde_json::to_value(&analysis).map_err(|e| err(format!("{e}")))?);
        }

        let stratum_dsp =
            stratum_dsp.ok_or_else(|| err("Missing stratum-dsp result".to_string()))?;

        let essentia_python = self.essentia_python_path();
        let essentia_available = essentia_python.is_some();
        let mut essentia: Option<serde_json::Value> = None;
        let mut essentia_cache_hit: Option<bool> = None;
        let mut essentia_error: Option<String> = None;

        if let Some(python_path) = essentia_python.as_deref() {
            if skip_cached {
                let store = self.internal_conn()?;
                if let Some(cached_entry) =
                    store::get_audio_analysis(&store, &file_path, "essentia")
                        .map_err(|e| err(format!("Cache read error: {e}")))?
                {
                    if cached_entry.file_size == file_size && cached_entry.file_mtime == file_mtime
                    {
                        essentia = Some(
                            serde_json::from_str(&cached_entry.features_json)
                                .map_err(|e| err(format!("Cache parse error: {e}")))?,
                        );
                        essentia_cache_hit = Some(true);
                    }
                }
            }

            if essentia.is_none() {
                match audio::run_essentia(python_path, &file_path).await {
                    Ok(features) => {
                        let analysis_version = features
                            .get("analyzer_version")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("unknown");
                        let features_json =
                            serde_json::to_string(&features).map_err(|e| err(format!("{e}")))?;
                        {
                            let store = self.internal_conn()?;
                            store::set_audio_analysis(
                                &store,
                                &file_path,
                                "essentia",
                                file_size,
                                file_mtime,
                                analysis_version,
                                &features_json,
                            )
                            .map_err(|e| err(format!("Cache write error: {e}")))?;
                        }
                        essentia = Some(features);
                        essentia_cache_hit = Some(false);
                    }
                    Err(e) => {
                        essentia_error = Some(e);
                    }
                }
            }
        }

        let result = serde_json::json!({
            "track_id": track.id,
            "title": track.title,
            "artist": track.artist,
            "stratum_dsp": stratum_dsp,
            "stratum_cache_hit": stratum_cache_hit,
            "essentia": essentia,
            "essentia_cache_hit": essentia_cache_hit,
            "essentia_available": essentia_available,
            "essentia_error": essentia_error,
        });
        let json = serde_json::to_string_pretty(&result).map_err(|e| err(format!("{e}")))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        description = "Batch analyze audio files with stratum-dsp and Essentia (when installed). Select tracks by IDs, playlist, or search filters. Results are cached."
    )]
    async fn analyze_audio_batch(
        &self,
        params: Parameters<AnalyzeAudioBatchParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let max_tracks = p.max_tracks.unwrap_or(20).min(200) as usize;
        let skip_cached = p.skip_cached.unwrap_or(true);

        let tracks = {
            let conn = self.conn()?;
            if let Some(ref ids) = p.track_ids {
                db::get_tracks_by_ids(&conn, ids).map_err(|e| err(format!("DB error: {e}")))?
            } else if let Some(ref playlist_id) = p.playlist_id {
                db::get_playlist_tracks(&conn, playlist_id, Some(max_tracks as u32))
                    .map_err(|e| err(format!("DB error: {e}")))?
            } else {
                let search = db::SearchParams {
                    query: p.query,
                    artist: p.artist,
                    genre: p.genre,
                    rating_min: p.rating_min,
                    bpm_min: p.bpm_min,
                    bpm_max: p.bpm_max,
                    key: p.key,
                    playlist: None,
                    has_genre: p.has_genre,
                    exclude_samples: true,
                    limit: Some(max_tracks as u32),
                };
                db::search_tracks(&conn, &search).map_err(|e| err(format!("DB error: {e}")))?
            }
        };

        let tracks: Vec<_> = tracks.into_iter().take(max_tracks).collect();
        let total = tracks.len();

        struct BatchTrackAnalysis {
            track_id: String,
            title: String,
            artist: String,
            file_path: String,
            file_size: i64,
            file_mtime: i64,
            stratum_dsp: serde_json::Value,
            stratum_cache_hit: bool,
            essentia: Option<serde_json::Value>,
            essentia_cache_hit: Option<bool>,
            essentia_error: Option<String>,
        }

        let mut analyzed = 0usize;
        let mut cached = 0usize;
        let mut essentia_analyzed = 0usize;
        let mut essentia_cached = 0usize;
        let mut essentia_failed = 0usize;
        let mut failed: Vec<serde_json::Value> = Vec::new();
        let mut rows: Vec<BatchTrackAnalysis> = Vec::new();

        for track in &tracks {
            let file_path = match resolve_file_path(&track.file_path) {
                Ok(path) => path,
                Err(e) => {
                    failed.push(serde_json::json!({
                        "track_id": track.id,
                        "artist": track.artist,
                        "title": track.title,
                        "analyzer": "stratum-dsp",
                        "error": format!("File path error: {e}"),
                    }));
                    continue;
                }
            };

            let metadata = match std::fs::metadata(&file_path) {
                Ok(metadata) => metadata,
                Err(e) => {
                    failed.push(serde_json::json!({
                        "track_id": track.id,
                        "artist": track.artist,
                        "title": track.title,
                        "analyzer": "stratum-dsp",
                        "error": format!("Cannot stat file: {e}"),
                    }));
                    continue;
                }
            };
            let file_size = metadata.len() as i64;
            let file_mtime = metadata
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);

            let mut stratum_dsp: Option<serde_json::Value> = None;
            let mut stratum_cache_hit = false;

            if skip_cached {
                let store = self.internal_conn()?;
                if let Some(cached_entry) =
                    store::get_audio_analysis(&store, &file_path, "stratum-dsp")
                        .map_err(|e| err(format!("Cache read error: {e}")))?
                {
                    if cached_entry.file_size == file_size && cached_entry.file_mtime == file_mtime
                    {
                        match serde_json::from_str::<serde_json::Value>(&cached_entry.features_json)
                        {
                            Ok(cached_json) => {
                                stratum_dsp = Some(cached_json);
                                stratum_cache_hit = true;
                                cached += 1;
                            }
                            Err(e) => {
                                failed.push(serde_json::json!({
                                    "track_id": track.id,
                                    "artist": track.artist,
                                    "title": track.title,
                                    "analyzer": "stratum-dsp",
                                    "error": format!("Cache parse error: {e}"),
                                }));
                                continue;
                            }
                        }
                    }
                }
            }

            if stratum_dsp.is_none() {
                let path_clone = file_path.clone();
                let decode_result =
                    tokio::task::spawn_blocking(move || audio::decode_to_samples(&path_clone))
                        .await;
                let (samples, sample_rate) = match decode_result {
                    Ok(Ok(value)) => value,
                    Ok(Err(e)) => {
                        failed.push(serde_json::json!({
                            "track_id": track.id,
                            "artist": track.artist,
                            "title": track.title,
                            "analyzer": "stratum-dsp",
                            "error": format!("Decode error: {e}"),
                        }));
                        continue;
                    }
                    Err(e) => {
                        failed.push(serde_json::json!({
                            "track_id": track.id,
                            "artist": track.artist,
                            "title": track.title,
                            "analyzer": "stratum-dsp",
                            "error": format!("Decode task failed: {e}"),
                        }));
                        continue;
                    }
                };

                let analysis_result =
                    tokio::task::spawn_blocking(move || audio::analyze(&samples, sample_rate))
                        .await;

                let analysis = match analysis_result {
                    Ok(Ok(analysis)) => analysis,
                    Ok(Err(e)) => {
                        failed.push(serde_json::json!({
                            "track_id": track.id,
                            "artist": track.artist,
                            "title": track.title,
                            "analyzer": "stratum-dsp",
                            "error": format!("Analysis error: {e}"),
                        }));
                        continue;
                    }
                    Err(e) => {
                        failed.push(serde_json::json!({
                            "track_id": track.id,
                            "artist": track.artist,
                            "title": track.title,
                            "analyzer": "stratum-dsp",
                            "error": format!("Analysis task failed: {e}"),
                        }));
                        continue;
                    }
                };

                let features_json =
                    serde_json::to_string(&analysis).map_err(|e| err(format!("{e}")))?;
                {
                    let store = self.internal_conn()?;
                    store::set_audio_analysis(
                        &store,
                        &file_path,
                        "stratum-dsp",
                        file_size,
                        file_mtime,
                        &analysis.analyzer_version,
                        &features_json,
                    )
                    .map_err(|e| err(format!("Cache write error: {e}")))?;
                }

                stratum_dsp =
                    Some(serde_json::to_value(&analysis).map_err(|e| err(format!("{e}")))?);
                analyzed += 1;
            }

            rows.push(BatchTrackAnalysis {
                track_id: track.id.clone(),
                title: track.title.clone(),
                artist: track.artist.clone(),
                file_path,
                file_size,
                file_mtime,
                stratum_dsp: stratum_dsp
                    .ok_or_else(|| err("Missing stratum-dsp result in batch".to_string()))?,
                stratum_cache_hit,
                essentia: None,
                essentia_cache_hit: None,
                essentia_error: None,
            });
        }

        let essentia_python = self.essentia_python_path();
        let essentia_available = essentia_python.is_some();

        if let Some(python_path) = essentia_python.as_deref() {
            for row in &mut rows {
                if skip_cached {
                    let store = self.internal_conn()?;
                    if let Some(cached_entry) =
                        store::get_audio_analysis(&store, &row.file_path, "essentia")
                            .map_err(|e| err(format!("Cache read error: {e}")))?
                    {
                        if cached_entry.file_size == row.file_size
                            && cached_entry.file_mtime == row.file_mtime
                        {
                            match serde_json::from_str::<serde_json::Value>(
                                &cached_entry.features_json,
                            ) {
                                Ok(cached_json) => {
                                    row.essentia = Some(cached_json);
                                    row.essentia_cache_hit = Some(true);
                                    essentia_cached += 1;
                                    continue;
                                }
                                Err(e) => {
                                    row.essentia_error = Some(format!("Cache parse error: {e}"));
                                    essentia_failed += 1;
                                    failed.push(serde_json::json!({
                                        "track_id": &row.track_id,
                                        "artist": &row.artist,
                                        "title": &row.title,
                                        "analyzer": "essentia",
                                        "error": format!("Cache parse error: {e}"),
                                    }));
                                    continue;
                                }
                            }
                        }
                    }
                }

                match audio::run_essentia(python_path, &row.file_path).await {
                    Ok(features) => {
                        let analysis_version = features
                            .get("analyzer_version")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("unknown");
                        let features_json =
                            serde_json::to_string(&features).map_err(|e| err(format!("{e}")))?;
                        {
                            let store = self.internal_conn()?;
                            store::set_audio_analysis(
                                &store,
                                &row.file_path,
                                "essentia",
                                row.file_size,
                                row.file_mtime,
                                analysis_version,
                                &features_json,
                            )
                            .map_err(|e| err(format!("Cache write error: {e}")))?;
                        }
                        row.essentia = Some(features);
                        row.essentia_cache_hit = Some(false);
                        essentia_analyzed += 1;
                    }
                    Err(e) => {
                        row.essentia_error = Some(e.clone());
                        essentia_failed += 1;
                        failed.push(serde_json::json!({
                            "track_id": &row.track_id,
                            "artist": &row.artist,
                            "title": &row.title,
                            "analyzer": "essentia",
                            "error": e,
                        }));
                    }
                }
            }
        }

        let results: Vec<serde_json::Value> = rows
            .into_iter()
            .map(|row| {
                serde_json::json!({
                    "track_id": row.track_id,
                    "title": row.title,
                    "artist": row.artist,
                    "stratum_dsp": row.stratum_dsp,
                    "stratum_cache_hit": row.stratum_cache_hit,
                    "essentia": row.essentia,
                    "essentia_cache_hit": row.essentia_cache_hit,
                    "essentia_available": essentia_available,
                    "essentia_error": row.essentia_error,
                })
            })
            .collect();

        let result = serde_json::json!({
            "summary": {
                "total": total,
                "analyzed": analyzed,
                "cached": cached,
                "failed": failed.len(),
                "essentia_available": essentia_available,
                "essentia_analyzed": essentia_analyzed,
                "essentia_cached": essentia_cached,
                "essentia_failed": essentia_failed,
            },
            "results": results,
            "failures": failed,
        });
        let json = serde_json::to_string_pretty(&result).map_err(|e| err(format!("{e}")))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        description = "Get all available data for a track in one call: Rekordbox metadata, cached audio analysis, cached enrichment, staged changes, and genre taxonomy mappings. Cache-only — never triggers external calls."
    )]
    async fn resolve_track_data(
        &self,
        params: Parameters<ResolveTrackDataParams>,
    ) -> Result<CallToolResult, McpError> {
        let track = {
            let conn = self.conn()?;
            db::get_track(&conn, &params.0.track_id)
                .map_err(|e| err(format!("DB error: {e}")))?
                .ok_or_else(|| {
                    McpError::invalid_params(
                        format!("Track '{}' not found", params.0.track_id),
                        None,
                    )
                })?
        };

        let norm_artist = discogs::normalize(&track.artist);
        let norm_title = discogs::normalize(&track.title);

        let essentia_installed = self.essentia_python_path().is_some();

        let (discogs_cache, beatport_cache, stratum_cache, essentia_cache) = {
            let store = self.internal_conn()?;
            let dc = store::get_enrichment(&store, "discogs", &norm_artist, &norm_title)
                .map_err(|e| err(format!("Cache read error: {e}")))?;
            let bc = store::get_enrichment(&store, "beatport", &norm_artist, &norm_title)
                .map_err(|e| err(format!("Cache read error: {e}")))?;
            let audio_cache_key =
                resolve_file_path(&track.file_path).unwrap_or_else(|_| track.file_path.clone());
            let sc = store::get_audio_analysis(&store, &audio_cache_key, "stratum-dsp")
                .map_err(|e| err(format!("Cache read error: {e}")))?;
            let ec = store::get_audio_analysis(&store, &audio_cache_key, "essentia")
                .map_err(|e| err(format!("Cache read error: {e}")))?;
            (dc, bc, sc, ec)
        };

        let staged = self.state.changes.get(&track.id);

        let result = resolve_single_track(
            &track,
            discogs_cache.as_ref(),
            beatport_cache.as_ref(),
            stratum_cache.as_ref(),
            essentia_cache.as_ref(),
            essentia_installed,
            staged.as_ref(),
        );

        let json = serde_json::to_string_pretty(&result).map_err(|e| err(format!("{e}")))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        description = "Get all available data for multiple tracks. Same as resolve_track_data but batched. Cache-only — never triggers external calls."
    )]
    async fn resolve_tracks_data(
        &self,
        params: Parameters<ResolveTracksDataParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let max_tracks = p.max_tracks.unwrap_or(50).min(200) as usize;

        let tracks = {
            let conn = self.conn()?;
            if let Some(ref ids) = p.track_ids {
                db::get_tracks_by_ids(&conn, ids).map_err(|e| err(format!("DB error: {e}")))?
            } else if let Some(ref playlist_id) = p.playlist_id {
                db::get_playlist_tracks(&conn, playlist_id, Some(max_tracks as u32))
                    .map_err(|e| err(format!("DB error: {e}")))?
            } else {
                let search = db::SearchParams {
                    query: p.query,
                    artist: p.artist,
                    genre: p.genre,
                    rating_min: p.rating_min,
                    bpm_min: p.bpm_min,
                    bpm_max: p.bpm_max,
                    key: p.key,
                    playlist: None,
                    has_genre: p.has_genre,
                    exclude_samples: true,
                    limit: Some(max_tracks as u32),
                };
                db::search_tracks(&conn, &search).map_err(|e| err(format!("DB error: {e}")))?
            }
        };

        let tracks: Vec<_> = tracks.into_iter().take(max_tracks).collect();

        let essentia_installed = self.essentia_python_path().is_some();
        let mut results = Vec::with_capacity(tracks.len());
        for track in &tracks {
            let norm_artist = discogs::normalize(&track.artist);
            let norm_title = discogs::normalize(&track.title);

            let (discogs_cache, beatport_cache, stratum_cache, essentia_cache) = {
                let store = self.internal_conn()?;
                let dc = store::get_enrichment(&store, "discogs", &norm_artist, &norm_title)
                    .map_err(|e| err(format!("Cache read error: {e}")))?;
                let bc = store::get_enrichment(&store, "beatport", &norm_artist, &norm_title)
                    .map_err(|e| err(format!("Cache read error: {e}")))?;
                let audio_cache_key =
                    resolve_file_path(&track.file_path).unwrap_or_else(|_| track.file_path.clone());
                let sc = store::get_audio_analysis(&store, &audio_cache_key, "stratum-dsp")
                    .map_err(|e| err(format!("Cache read error: {e}")))?;
                let ec = store::get_audio_analysis(&store, &audio_cache_key, "essentia")
                    .map_err(|e| err(format!("Cache read error: {e}")))?;
                (dc, bc, sc, ec)
            };

            let staged = self.state.changes.get(&track.id);

            results.push(resolve_single_track(
                track,
                discogs_cache.as_ref(),
                beatport_cache.as_ref(),
                stratum_cache.as_ref(),
                essentia_cache.as_ref(),
                essentia_installed,
                staged.as_ref(),
            ));
        }

        let json = serde_json::to_string_pretty(&results).map_err(|e| err(format!("{e}")))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }
}

/// Map a genre/style string through the taxonomy.
/// Returns (maps_to, mapping_type) where mapping_type is "exact", "alias", or "unknown".
fn map_genre_through_taxonomy(style: &str) -> (Option<String>, &'static str) {
    if genre::is_known_genre(style) {
        // Find the canonical casing from the taxonomy
        let canonical = genre::GENRES
            .iter()
            .find(|g| g.eq_ignore_ascii_case(style))
            .unwrap_or(&style);
        (Some(canonical.to_string()), "exact")
    } else if let Some(canonical) = genre::normalize_genre(style) {
        (Some(canonical.to_string()), "alias")
    } else {
        (None, "unknown")
    }
}

/// Build the resolved JSON payload for a single track.
/// This is a pure function that takes pre-fetched data and produces the output.
pub(crate) fn resolve_single_track(
    track: &crate::types::Track,
    discogs_cache: Option<&store::CachedEnrichment>,
    beatport_cache: Option<&store::CachedEnrichment>,
    stratum_cache: Option<&store::CachedAudioAnalysis>,
    essentia_cache: Option<&store::CachedAudioAnalysis>,
    essentia_installed: bool,
    staged: Option<&crate::types::TrackChange>,
) -> serde_json::Value {
    // --- Rekordbox section ---
    let rekordbox = serde_json::json!({
        "title": track.title,
        "artist": track.artist,
        "remixer": track.remixer,
        "album": track.album,
        "genre": track.genre,
        "bpm": track.bpm,
        "key": track.key,
        "duration_s": track.length,
        "year": track.year,
        "rating": track.rating,
        "comments": track.comments,
        "label": track.label,
        "color": track.color,
        "play_count": track.play_count,
        "date_added": track.date_added,
    });

    // --- Audio analysis section ---
    let stratum_json = stratum_cache
        .and_then(|sc| serde_json::from_str::<serde_json::Value>(&sc.features_json).ok());
    let essentia_json = essentia_cache
        .and_then(|ec| serde_json::from_str::<serde_json::Value>(&ec.features_json).ok());

    let (bpm_agreement, key_agreement) = if let Some(ref sj) = stratum_json {
        let stratum_bpm = sj.get("bpm").and_then(|v| v.as_f64());
        let stratum_key = sj.get("key").and_then(|v| v.as_str());

        let bpm_agree = stratum_bpm.map(|sb| (sb - track.bpm).abs() <= 2.0);
        let key_agree = stratum_key.map(|sk| sk.eq_ignore_ascii_case(&track.key));

        (bpm_agree, key_agree)
    } else {
        (None, None)
    };

    let audio_analysis = if stratum_json.is_some() || essentia_json.is_some() {
        serde_json::json!({
            "stratum_dsp": stratum_json,
            "essentia": essentia_json,
            "bpm_agreement": bpm_agreement,
            "key_agreement": key_agreement,
        })
    } else {
        serde_json::Value::Null
    };

    // --- Enrichment sections ---
    let discogs_val = parse_enrichment_cache(discogs_cache);
    let beatport_val = parse_enrichment_cache(beatport_cache);

    // --- Staged changes ---
    let staged_val = staged.map(|s| {
        serde_json::json!({
            "genre": s.genre,
            "comments": s.comments,
            "rating": s.rating,
            "color": s.color,
        })
    });

    // --- Data completeness ---
    let data_completeness = serde_json::json!({
        "rekordbox": true,
        "stratum_dsp": stratum_cache.is_some(),
        "essentia": essentia_cache.is_some(),
        "essentia_installed": essentia_installed,
        "discogs": discogs_cache.is_some(),
        "beatport": beatport_cache.is_some(),
    });

    // --- Genre taxonomy mappings ---
    let current_genre_canonical = if track.genre.is_empty() {
        serde_json::Value::Null
    } else if genre::is_known_genre(&track.genre) {
        let canonical = genre::GENRES
            .iter()
            .find(|g| g.eq_ignore_ascii_case(&track.genre))
            .map(|g| g.to_string())
            .unwrap_or_else(|| track.genre.clone());
        serde_json::json!(canonical)
    } else if let Some(canonical) = genre::normalize_genre(&track.genre) {
        serde_json::json!(canonical)
    } else {
        serde_json::Value::Null
    };

    // Map Discogs styles through taxonomy
    let discogs_style_mappings: Vec<serde_json::Value> = discogs_val
        .as_ref()
        .and_then(|v| v.get("styles"))
        .and_then(|v| v.as_array())
        .map(|styles| {
            styles
                .iter()
                .filter_map(|s| s.as_str())
                .map(|style| {
                    let (maps_to, mapping_type) = map_genre_through_taxonomy(style);
                    serde_json::json!({
                        "style": style,
                        "maps_to": maps_to,
                        "mapping_type": mapping_type,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    // Map Beatport genre through taxonomy
    let beatport_genre_mapping = beatport_val
        .as_ref()
        .and_then(|v| v.get("genre"))
        .and_then(|v| v.as_str())
        .filter(|g| !g.is_empty())
        .map(|bp_genre| {
            let (maps_to, mapping_type) = map_genre_through_taxonomy(bp_genre);
            serde_json::json!({
                "genre": bp_genre,
                "maps_to": maps_to,
                "mapping_type": mapping_type,
            })
        });

    let genre_taxonomy = serde_json::json!({
        "current_genre_canonical": current_genre_canonical,
        "discogs_style_mappings": discogs_style_mappings,
        "beatport_genre_mapping": beatport_genre_mapping,
    });

    serde_json::json!({
        "track_id": track.id,
        "rekordbox": rekordbox,
        "audio_analysis": audio_analysis,
        "discogs": discogs_val,
        "beatport": beatport_val,
        "staged_changes": staged_val,
        "data_completeness": data_completeness,
        "genre_taxonomy": genre_taxonomy,
    })
}

/// Parse a cached enrichment entry's response_json into a serde_json::Value.
/// Returns None if cache entry is None or has no response_json.
/// Injects match_quality and cached_at metadata into the returned object.
fn parse_enrichment_cache(cache: Option<&store::CachedEnrichment>) -> Option<serde_json::Value> {
    cache.and_then(|c| {
        let mut val = c
            .response_json
            .as_ref()
            .and_then(|json_str| serde_json::from_str::<serde_json::Value>(json_str).ok())?;
        if let serde_json::Value::Object(ref mut map) = val {
            map.insert("match_quality".into(), serde_json::json!(c.match_quality));
            map.insert("cached_at".into(), serde_json::json!(c.created_at));
        }
        Some(val)
    })
}

/// Resolve a Rekordbox file path to an actual filesystem path.
/// Tries the raw path first; if that fails, tries percent-decoding.
fn resolve_file_path(raw_path: &str) -> Result<String, McpError> {
    // Try raw path first
    if std::fs::metadata(raw_path).is_ok() {
        return Ok(raw_path.to_string());
    }

    // Try percent-decoded
    let decoded = percent_encoding::percent_decode_str(raw_path)
        .decode_utf8()
        .map_err(|e| err(format!("Invalid UTF-8 in file path: {e}")))?
        .to_string();

    if std::fs::metadata(&decoded).is_ok() {
        return Ok(decoded);
    }

    Err(err(format!(
        "File not found (tried raw and decoded): {raw_path}"
    )))
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
    use std::collections::{HashMap, HashSet};
    use std::sync::{Arc, Mutex, OnceLock};

    use rmcp::ServiceExt;
    use rmcp::model::CallToolRequestParams;
    use rusqlite::{Connection, params};
    use serde::Deserialize;
    use tempfile::TempDir;

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

    async fn call_tool_via_router(
        tool_name: &str,
        arguments: Option<serde_json::Map<String, serde_json::Value>>,
    ) -> CallToolResult {
        let (client_io, server_io) = tokio::io::duplex(64 * 1024);
        let (server_result, client_result) = tokio::join!(
            CrateDigServer::new(None).serve(server_io),
            ().serve(client_io)
        );
        let mut server = server_result.expect("server should start over in-memory transport");
        let mut client = client_result.expect("client should connect over in-memory transport");

        let result = client
            .call_tool(CallToolRequestParams {
                meta: None,
                name: tool_name.to_owned().into(),
                arguments,
                task: None,
            })
            .await
            .expect("tool call through router should succeed");

        client
            .close()
            .await
            .expect("client should close cleanly after tool call");
        server
            .close()
            .await
            .expect("server should close cleanly after tool call");

        result
    }

    const GOLDEN_GENRES_FIXTURE_PATH: &str = "tests/fixtures/golden_genres.json";

    #[derive(Debug, Deserialize)]
    struct GoldenGenreFixtureEntry {
        artist: String,
        title: String,
        expected_genre: String,
        notes: String,
    }

    fn default_http_client_for_tests() -> reqwest::Client {
        reqwest::Client::builder()
            .user_agent("CrateGuide/2.0")
            .build()
            .expect("default test HTTP client should build")
    }

    fn create_server_with_connections(
        db_conn: Connection,
        store_conn: Connection,
        http: reqwest::Client,
    ) -> CrateDigServer {
        let server = CrateDigServer {
            state: Arc::new(ServerState {
                db: OnceLock::new(),
                internal_db: OnceLock::new(),
                essentia_python: OnceLock::new(),
                db_path: None,
                changes: ChangeManager::new(),
                http,
            }),
            tool_router: CrateDigServer::tool_router(),
        };

        server
            .state
            .db
            .set(Ok(Mutex::new(db_conn)))
            .expect("test DB should initialize exactly once");
        server
            .state
            .internal_db
            .set(Ok(Mutex::new(store_conn)))
            .expect("test internal store should initialize exactly once");

        server
    }

    fn create_real_server_with_temp_store(
        http: reqwest::Client,
    ) -> Option<(CrateDigServer, TempDir)> {
        let db_conn = db::open_real_db()?;
        let store_dir = tempfile::tempdir().ok()?;
        let store_path = store_dir.path().join("internal.sqlite3");
        let store_conn = store::open(
            store_path
                .to_str()
                .expect("temp store path should be UTF-8"),
        )
        .expect("internal store should open for integration test");

        let server = create_server_with_connections(db_conn, store_conn, http);
        Some((server, store_dir))
    }

    fn sample_real_tracks(server: &CrateDigServer, limit: u32) -> Vec<crate::types::Track> {
        let conn = server
            .conn()
            .expect("real DB connection should be available for integration test");
        db::search_tracks(
            &conn,
            &db::SearchParams {
                query: None,
                artist: None,
                genre: None,
                rating_min: None,
                bpm_min: None,
                bpm_max: None,
                key: None,
                playlist: None,
                has_genre: Some(true),
                exclude_samples: true,
                limit: Some(limit),
            },
        )
        .expect("sample search should succeed")
        .into_iter()
        .filter(|t| !t.artist.trim().is_empty() && !t.title.trim().is_empty())
        .collect()
    }

    fn create_single_track_test_db(track_id: &str, raw_file_path: &str) -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory DB should open");
        conn.execute_batch(
            "
            CREATE TABLE djmdArtist (
                ID VARCHAR(255) PRIMARY KEY,
                Name VARCHAR(255),
                rb_local_deleted INTEGER DEFAULT 0
            );
            CREATE TABLE djmdAlbum (
                ID VARCHAR(255) PRIMARY KEY,
                Name VARCHAR(255),
                rb_local_deleted INTEGER DEFAULT 0
            );
            CREATE TABLE djmdGenre (
                ID VARCHAR(255) PRIMARY KEY,
                Name VARCHAR(255),
                rb_local_deleted INTEGER DEFAULT 0
            );
            CREATE TABLE djmdKey (
                ID VARCHAR(255) PRIMARY KEY,
                ScaleName VARCHAR(255),
                rb_local_deleted INTEGER DEFAULT 0
            );
            CREATE TABLE djmdLabel (
                ID VARCHAR(255) PRIMARY KEY,
                Name VARCHAR(255),
                rb_local_deleted INTEGER DEFAULT 0
            );
            CREATE TABLE djmdColor (
                ID VARCHAR(255) PRIMARY KEY,
                ColorCode INTEGER,
                Commnt VARCHAR(255),
                rb_local_deleted INTEGER DEFAULT 0
            );
            CREATE TABLE djmdContent (
                ID VARCHAR(255) PRIMARY KEY,
                Title VARCHAR(255),
                ArtistID VARCHAR(255),
                AlbumID VARCHAR(255),
                GenreID VARCHAR(255),
                KeyID VARCHAR(255),
                ColorID VARCHAR(255),
                LabelID VARCHAR(255),
                RemixerID VARCHAR(255),
                BPM INTEGER DEFAULT 0,
                Rating INTEGER DEFAULT 0,
                Commnt TEXT DEFAULT '',
                ReleaseYear INTEGER DEFAULT 0,
                Length INTEGER DEFAULT 0,
                FolderPath VARCHAR(255) DEFAULT '',
                DJPlayCount VARCHAR(255) DEFAULT '0',
                BitRate INTEGER DEFAULT 0,
                SampleRate INTEGER DEFAULT 0,
                FileType INTEGER DEFAULT 0,
                created_at TEXT DEFAULT '',
                rb_local_deleted INTEGER DEFAULT 0
            );

            INSERT INTO djmdArtist (ID, Name) VALUES ('a1', 'Aníbal');
            INSERT INTO djmdAlbum (ID, Name) VALUES ('al1', 'Encoded Paths');
            INSERT INTO djmdGenre (ID, Name) VALUES ('g1', 'Deep House');
            INSERT INTO djmdKey (ID, ScaleName) VALUES ('k1', 'Am');
            INSERT INTO djmdLabel (ID, Name) VALUES ('l1', 'Test Label');
            INSERT INTO djmdColor (ID, ColorCode, Commnt) VALUES ('c1', 16711935, 'Rose');
            ",
        )
        .expect("test schema should initialize");

        conn.execute(
            "INSERT INTO djmdContent (
                ID, Title, ArtistID, AlbumID, GenreID, KeyID, ColorID, LabelID, RemixerID,
                BPM, Rating, Commnt, ReleaseYear, Length, FolderPath, DJPlayCount, BitRate,
                SampleRate, FileType, created_at, rb_local_deleted
            ) VALUES (
                ?1, 'Señorita', 'a1', 'al1', 'g1', 'k1', 'c1', 'l1', '',
                12800, 204, 'percent path test', 2025, 240, ?2, '0', 1411,
                44100, 5, '2025-01-01', 0
            )",
            params![track_id, raw_file_path],
        )
        .expect("test track should insert");

        conn
    }

    fn canonical_genre_name(raw_genre: &str) -> String {
        if let Some(canonical) = genre::GENRES
            .iter()
            .find(|g| g.eq_ignore_ascii_case(raw_genre))
        {
            return canonical.to_string();
        }
        if let Some(alias_target) = genre::normalize_genre(raw_genre) {
            return alias_target.to_string();
        }
        raw_genre.to_string()
    }

    fn load_golden_genres_fixture() -> Vec<GoldenGenreFixtureEntry> {
        let raw = std::fs::read_to_string(GOLDEN_GENRES_FIXTURE_PATH)
            .expect("golden genres fixture should be readable");
        serde_json::from_str(&raw).expect("golden genres fixture should be valid JSON")
    }

    fn find_track_by_artist_and_title(
        conn: &Connection,
        artist: &str,
        title: &str,
    ) -> Option<crate::types::Track> {
        let sql = format!(
            "{}
             WHERE c.rb_local_deleted = 0
               AND lower(COALESCE(a.Name, '')) = lower(?1)
               AND lower(COALESCE(c.Title, '')) = lower(?2)
             LIMIT 1",
            db::TRACK_SELECT
        );
        let mut stmt = conn
            .prepare(&sql)
            .expect("fixture lookup query should prepare");
        let mut rows = stmt
            .query_map(params![artist, title], db::row_to_track)
            .expect("fixture lookup query should run");
        match rows.next() {
            Some(Ok(track)) => Some(track),
            Some(Err(e)) => panic!("fixture lookup failed for {artist} - {title}: {e}"),
            None => None,
        }
    }

    #[test]
    #[cfg(unix)]
    fn probe_essentia_python_prefers_env_override_when_valid() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("temp dir should create");
        let fake_python = dir.path().join("fake-python");
        std::fs::write(&fake_python, "#!/bin/sh\nexit 0\n")
            .expect("fake python script should be written");
        let mut perms = std::fs::metadata(&fake_python)
            .expect("fake python metadata should be readable")
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&fake_python, perms)
            .expect("fake python script should be executable");

        let resolved = probe_essentia_python_from_sources(
            fake_python.to_str(),
            Some(dir.path().join("missing-default-python")),
        );

        assert_eq!(
            resolved.as_deref(),
            fake_python.to_str(),
            "valid env override should win over default candidate"
        );
    }

    #[tokio::test]
    async fn analyze_track_audio_reports_essentia_unavailable_when_probe_is_none() {
        let audio_dir = tempfile::tempdir().expect("temp audio dir should create");
        let audio_path = audio_dir.path().join("cached-track.flac");
        std::fs::write(&audio_path, b"fake-audio-data").expect("temp audio file should be created");
        let audio_path_str = audio_path.to_string_lossy().to_string();

        let metadata = std::fs::metadata(&audio_path).expect("temp audio metadata should load");
        let file_size = metadata.len() as i64;
        let file_mtime = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        let db_conn = create_single_track_test_db("essentia-missing-1", &audio_path_str);
        let store_dir = tempfile::tempdir().expect("temp store dir should create");
        let store_path = store_dir.path().join("internal.sqlite3");
        let store_conn = store::open(
            store_path
                .to_str()
                .expect("temp store path should be UTF-8"),
        )
        .expect("temp internal store should open");
        store::set_audio_analysis(
            &store_conn,
            &audio_path_str,
            "stratum-dsp",
            file_size,
            file_mtime,
            "stratum-dsp-1.0.0",
            r#"{"bpm":128.0,"key":"Am","analyzer_version":"stratum-dsp-1.0.0"}"#,
        )
        .expect("stratum cache should be seeded");

        let server =
            create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());
        server
            .state
            .essentia_python
            .set(None)
            .expect("essentia probe state should be seeded once");

        let result = server
            .analyze_track_audio(Parameters(AnalyzeTrackAudioParams {
                track_id: "essentia-missing-1".to_string(),
                skip_cached: Some(true),
            }))
            .await
            .expect("analyze_track_audio should succeed with cached stratum data");
        let payload = extract_json(&result);

        assert_eq!(payload["essentia_available"], false);
        assert!(
            payload["essentia"].is_null(),
            "essentia payload should be null when probe is unavailable"
        );
        assert_eq!(
            payload["stratum_cache_hit"], true,
            "stratum cache should still be used when Essentia is unavailable"
        );
        assert!(
            payload["stratum_dsp"].is_object(),
            "stratum_dsp should still be returned"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn analyze_track_audio_essentia_cache_round_trip_real_track() {
        let Some((server, _store_dir)) =
            create_real_server_with_temp_store(default_http_client_for_tests())
        else {
            eprintln!("Skipping: backup tarball not found (set REKORDBOX_TEST_BACKUP)");
            return;
        };

        if server.essentia_python_path().is_none() {
            eprintln!("Skipping: Essentia Python not available");
            return;
        }

        let track = sample_real_tracks(&server, 20)
            .into_iter()
            .find(|t| resolve_file_path(&t.file_path).is_ok())
            .expect("integration test needs at least one track with accessible audio file");

        let first = server
            .analyze_track_audio(Parameters(AnalyzeTrackAudioParams {
                track_id: track.id.clone(),
                skip_cached: Some(false),
            }))
            .await
            .expect("initial analysis should succeed");
        let first_payload = extract_json(&first);
        assert_eq!(first_payload["essentia_available"], true);
        assert!(
            first_payload["essentia"].is_object(),
            "real Essentia run should produce feature JSON"
        );
        assert_eq!(first_payload["essentia_cache_hit"], false);

        let second = server
            .analyze_track_audio(Parameters(AnalyzeTrackAudioParams {
                track_id: track.id,
                skip_cached: Some(true),
            }))
            .await
            .expect("cached analysis should succeed");
        let second_payload = extract_json(&second);
        assert_eq!(second_payload["essentia_available"], true);
        assert_eq!(second_payload["stratum_cache_hit"], true);
        assert_eq!(second_payload["essentia_cache_hit"], true);
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
    async fn write_xml_no_change_path_via_router_includes_provenance() {
        let result = call_tool_via_router("write_xml", None).await;
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

    #[tokio::test]
    async fn update_tracks_via_router_includes_provenance() {
        let result = call_tool_via_router(
            "update_tracks",
            serde_json::json!({
                "changes": [{
                    "track_id": "router-test-track-1",
                    "genre": "NotInTaxonomy"
                }]
            })
            .as_object()
            .cloned(),
        )
        .await;

        let payload = extract_json(&result);
        assert_eq!(
            payload
                .get("staged")
                .and_then(serde_json::Value::as_u64)
                .expect("staged should be present"),
            1
        );
        let warnings = payload
            .get("warnings")
            .and_then(serde_json::Value::as_array)
            .expect("warnings should be present for non-taxonomy genre");
        assert!(
            !warnings.is_empty(),
            "warnings should include at least one non-taxonomy genre warning"
        );
        assert_has_provenance(&payload);
    }

    #[tokio::test]
    async fn get_genre_taxonomy_via_router_includes_provenance() {
        let result = call_tool_via_router("get_genre_taxonomy", None).await;
        let payload = extract_json(&result);

        let genres = payload
            .get("genres")
            .and_then(serde_json::Value::as_array)
            .expect("genres should be present");
        assert!(
            !genres.is_empty(),
            "genres should include configured taxonomy entries"
        );
        assert_has_provenance(&payload);
    }

    #[tokio::test]
    async fn enrich_tracks_invalid_provider_returns_error() {
        let server = CrateDigServer::new(None);

        let err = server
            .enrich_tracks(Parameters(EnrichTracksParams {
                track_ids: None,
                playlist_id: None,
                query: None,
                artist: None,
                genre: None,
                has_genre: None,
                bpm_min: None,
                bpm_max: None,
                key: None,
                rating_min: None,
                max_tracks: Some(1),
                providers: Some(vec!["spotify".to_string()]),
                skip_cached: Some(true),
                force_refresh: Some(false),
            }))
            .await
            .expect_err("invalid provider should fail validation");

        assert!(
            format!("{err}").contains("unknown provider 'spotify'"),
            "error should mention invalid provider, got: {err}"
        );
    }

    #[tokio::test]
    async fn enrich_tracks_discogs_skip_cached_reports_cached_counts() {
        let db_conn = create_single_track_test_db("cached-track-1", "/tmp/cached-track-1.flac");
        db_conn
            .execute(
                "INSERT INTO djmdContent (
                    ID, Title, ArtistID, AlbumID, GenreID, KeyID, ColorID, LabelID, RemixerID,
                    BPM, Rating, Commnt, ReleaseYear, Length, FolderPath, DJPlayCount, BitRate,
                    SampleRate, FileType, created_at, rb_local_deleted
                ) VALUES (
                    ?1, ?2, 'a1', 'al1', 'g1', 'k1', 'c1', 'l1', '',
                    12700, 153, 'cached batch test', 2025, 230, ?3, '0', 1411,
                    44100, 5, '2025-01-01', 0
                )",
                params![
                    "cached-track-2",
                    "Corazón Cached",
                    "/tmp/cached-track-2.flac"
                ],
            )
            .expect("second test track should insert");

        let store_dir = tempfile::tempdir().expect("temp store dir should create");
        let store_path = store_dir.path().join("internal.sqlite3");
        let store_conn = store::open(
            store_path
                .to_str()
                .expect("temp store path should be UTF-8"),
        )
        .expect("temp internal store should open");

        let artist = "Aníbal";
        let title_one = "Señorita";
        let title_two = "Corazón Cached";
        let norm_artist = discogs::normalize(artist);
        let norm_title_one = discogs::normalize(title_one);
        let norm_title_two = discogs::normalize(title_two);

        let cached_one = serde_json::json!({
            "title": "Anibal - Senorita",
            "genres": ["Electronic"],
            "styles": ["Deep House"],
            "fuzzy_match": false
        })
        .to_string();
        let cached_two = serde_json::json!({
            "title": "Anibal - Corazon Cached",
            "genres": ["Electronic"],
            "styles": ["House"],
            "fuzzy_match": false
        })
        .to_string();

        store::set_enrichment(
            &store_conn,
            "discogs",
            &norm_artist,
            &norm_title_one,
            Some("exact"),
            Some(&cached_one),
        )
        .expect("first sentinel discogs cache entry should write");
        store::set_enrichment(
            &store_conn,
            "discogs",
            &norm_artist,
            &norm_title_two,
            Some("exact"),
            Some(&cached_two),
        )
        .expect("second sentinel discogs cache entry should write");

        let server =
            create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());

        let params = EnrichTracksParams {
            track_ids: Some(vec![
                "cached-track-1".to_string(),
                "cached-track-2".to_string(),
            ]),
            playlist_id: None,
            query: None,
            artist: None,
            genre: None,
            has_genre: None,
            bpm_min: None,
            bpm_max: None,
            key: None,
            rating_min: None,
            max_tracks: Some(10),
            providers: Some(vec!["discogs".to_string()]),
            skip_cached: Some(true),
            force_refresh: Some(false),
        };

        let first_result = server
            .enrich_tracks(Parameters(params))
            .await
            .expect("enrich_tracks should succeed when everything is cached");
        let first_payload = extract_json(&first_result);
        assert_eq!(first_payload["summary"]["total"], 2);
        assert_eq!(first_payload["summary"]["enriched"], 0);
        assert_eq!(first_payload["summary"]["cached"], 2);
        assert_eq!(first_payload["summary"]["skipped"], 0);
        assert_eq!(first_payload["summary"]["failed"], 0);
        assert_eq!(
            first_payload["failures"]
                .as_array()
                .expect("failures should be an array")
                .len(),
            0
        );

        let second_result = server
            .enrich_tracks(Parameters(EnrichTracksParams {
                track_ids: Some(vec![
                    "cached-track-1".to_string(),
                    "cached-track-2".to_string(),
                ]),
                playlist_id: None,
                query: None,
                artist: None,
                genre: None,
                has_genre: None,
                bpm_min: None,
                bpm_max: None,
                key: None,
                rating_min: None,
                max_tracks: Some(10),
                providers: Some(vec!["discogs".to_string()]),
                skip_cached: Some(true),
                force_refresh: Some(false),
            }))
            .await
            .expect("second enrich_tracks run should also be fully cached");
        let second_payload = extract_json(&second_result);
        assert_eq!(second_payload["summary"]["total"], 2);
        assert_eq!(second_payload["summary"]["enriched"], 0);
        assert_eq!(second_payload["summary"]["cached"], 2);
        assert_eq!(second_payload["summary"]["skipped"], 0);
        assert_eq!(second_payload["summary"]["failed"], 0);

        let store = server
            .internal_conn()
            .expect("internal store should be available");
        let entry_one = store::get_enrichment(&store, "discogs", &norm_artist, &norm_title_one)
            .expect("cache read should succeed")
            .expect("first cache entry should still exist");
        let entry_two = store::get_enrichment(&store, "discogs", &norm_artist, &norm_title_two)
            .expect("cache read should succeed")
            .expect("second cache entry should still exist");
        assert_eq!(
            entry_one.response_json.as_deref(),
            Some(cached_one.as_str())
        );
        assert_eq!(
            entry_two.response_json.as_deref(),
            Some(cached_two.as_str())
        );
    }

    #[tokio::test]
    async fn resolve_track_data_uses_decoded_path_for_audio_cache_lookup() {
        let temp_audio_dir = tempfile::tempdir().expect("temp audio dir should create");
        let decoded_path = temp_audio_dir.path().join("Aníbal Track.flac");
        std::fs::write(&decoded_path, b"fake-audio-data")
            .expect("decoded path file should exist for resolve_file_path");

        let decoded_path_str = decoded_path.to_string_lossy().to_string();
        let raw_path = decoded_path_str
            .replace("Aníbal", "An%C3%ADbal")
            .replace(' ', "%20");
        assert_ne!(
            raw_path, decoded_path_str,
            "raw path must differ from decoded path for this regression test"
        );
        assert!(
            std::fs::metadata(&decoded_path_str).is_ok(),
            "decoded file path should exist"
        );
        assert!(
            std::fs::metadata(&raw_path).is_err(),
            "raw encoded path should not exist"
        );

        let db_conn = create_single_track_test_db("encoded-track-1", &raw_path);
        let store_dir = tempfile::tempdir().expect("temp store dir should create");
        let store_path = store_dir.path().join("internal.sqlite3");
        let store_conn = store::open(
            store_path
                .to_str()
                .expect("temp store path should be UTF-8"),
        )
        .expect("temp internal store should open");

        store::set_audio_analysis(
            &store_conn,
            &decoded_path_str,
            "stratum-dsp",
            12,
            1_700_000_000,
            "stratum-dsp-1.0.0",
            r#"{"bpm":128.0,"key":"Am","analyzer_version":"stratum-dsp-1.0.0"}"#,
        )
        .expect("audio cache entry should write with decoded cache key");

        let server =
            create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());
        let result = server
            .resolve_track_data(Parameters(ResolveTrackDataParams {
                track_id: "encoded-track-1".to_string(),
            }))
            .await
            .expect("resolve_track_data should succeed");
        let payload = extract_json(&result);

        assert_eq!(
            payload["data_completeness"]["stratum_dsp"], true,
            "decoded path lookup should find stratum cache entry"
        );
        assert!(
            payload["audio_analysis"]["stratum_dsp"].is_object(),
            "audio_analysis.stratum_dsp should be populated from cache"
        );
        assert_eq!(payload["audio_analysis"]["stratum_dsp"]["key"], "Am");
    }

    #[tokio::test]
    #[ignore]
    async fn force_refresh_bypasses_enrichment_cache() {
        let offline_http = reqwest::Client::builder()
            .user_agent("CrateGuide/2.0")
            .proxy(
                reqwest::Proxy::all("http://127.0.0.1:9").expect("offline proxy URL should parse"),
            )
            .build()
            .expect("offline HTTP client should build");

        let Some((server, _store_dir)) = create_real_server_with_temp_store(offline_http) else {
            eprintln!("Skipping: backup tarball not found (set REKORDBOX_TEST_BACKUP)");
            return;
        };

        let track = sample_real_tracks(&server, 1)
            .into_iter()
            .next()
            .expect("integration test needs at least one real track");
        let norm_artist = discogs::normalize(&track.artist);
        let norm_title = discogs::normalize(&track.title);
        let cached_json = serde_json::json!({"genre":"Sentinel Genre","key":"Am","bpm":128});
        let cached_json_str = cached_json.to_string();

        {
            let store = server
                .internal_conn()
                .expect("internal store should be available");
            store::set_enrichment(
                &store,
                "beatport",
                &norm_artist,
                &norm_title,
                Some("exact"),
                Some(&cached_json_str),
            )
            .expect("sentinel cache entry should write");
        }

        let cache_hit = server
            .lookup_beatport(Parameters(LookupBeatportParams {
                artist: track.artist.clone(),
                title: track.title.clone(),
                force_refresh: Some(false),
            }))
            .await
            .expect("lookup_beatport(force_refresh=false) should return cache");
        let cache_hit_json = extract_json(&cache_hit);
        assert_eq!(cache_hit_json["cache_hit"], true);
        assert_eq!(cache_hit_json["genre"], "Sentinel Genre");

        let refresh_err = server
            .lookup_beatport(Parameters(LookupBeatportParams {
                artist: track.artist.clone(),
                title: track.title.clone(),
                force_refresh: Some(true),
            }))
            .await
            .expect_err("force_refresh=true should bypass cache and attempt HTTP call");
        assert!(
            format!("{refresh_err}").contains("Beatport error"),
            "force refresh should fail via offline HTTP path, got: {refresh_err}"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn enrich_tracks_beatport_schema_matches_individual_lookup() {
        let Some((server, _store_dir)) =
            create_real_server_with_temp_store(default_http_client_for_tests())
        else {
            eprintln!("Skipping: backup tarball not found (set REKORDBOX_TEST_BACKUP)");
            return;
        };

        let candidates = sample_real_tracks(&server, 30);
        assert!(
            !candidates.is_empty(),
            "integration test needs candidate tracks from real DB"
        );

        let mut selected_track: Option<crate::types::Track> = None;
        for track in candidates.into_iter().take(10) {
            let lookup = server
                .lookup_beatport(Parameters(LookupBeatportParams {
                    artist: track.artist.clone(),
                    title: track.title.clone(),
                    force_refresh: Some(true),
                }))
                .await;

            let Ok(result) = lookup else {
                continue;
            };
            let payload = extract_json(&result);
            if payload
                .get("genre")
                .and_then(serde_json::Value::as_str)
                .is_some()
            {
                selected_track = Some(track);
                break;
            }
        }

        let track = selected_track.expect(
            "could not find a track with a successful Beatport match; \
             rerun when network/providers are available",
        );
        let norm_artist = discogs::normalize(&track.artist);
        let norm_title = discogs::normalize(&track.title);

        let individual_cache = {
            let store = server
                .internal_conn()
                .expect("internal store should be available");
            store::get_enrichment(&store, "beatport", &norm_artist, &norm_title)
                .expect("cache read should succeed")
                .expect("individual lookup should have created cache entry")
        };
        let individual_json: serde_json::Value = serde_json::from_str(
            individual_cache
                .response_json
                .as_deref()
                .expect("individual beatport cache should contain JSON"),
        )
        .expect("individual beatport cache JSON should parse");
        assert!(
            individual_json
                .get("genre")
                .and_then(serde_json::Value::as_str)
                .is_some(),
            "individual beatport cache should have string 'genre'"
        );
        let individual_fields: HashSet<String> = individual_json
            .as_object()
            .expect("individual beatport cache should be object")
            .keys()
            .cloned()
            .collect();

        {
            let store = server
                .internal_conn()
                .expect("internal store should be available");
            store
                .execute(
                    "DELETE FROM enrichment_cache
                     WHERE provider = ?1 AND query_artist = ?2 AND query_title = ?3",
                    params!["beatport", &norm_artist, &norm_title],
                )
                .expect("cache clear should succeed");
        }

        let enrich_result = server
            .enrich_tracks(Parameters(EnrichTracksParams {
                track_ids: Some(vec![track.id.clone()]),
                playlist_id: None,
                query: None,
                artist: None,
                genre: None,
                has_genre: None,
                bpm_min: None,
                bpm_max: None,
                key: None,
                rating_min: None,
                max_tracks: Some(1),
                providers: Some(vec!["beatport".to_string()]),
                skip_cached: Some(false),
                force_refresh: Some(true),
            }))
            .await
            .expect("enrich_tracks should succeed for beatport provider");
        let enrich_payload = extract_json(&enrich_result);
        assert_eq!(enrich_payload["summary"]["total"], 1);

        let batch_cache = {
            let store = server
                .internal_conn()
                .expect("internal store should be available");
            store::get_enrichment(&store, "beatport", &norm_artist, &norm_title)
                .expect("cache read should succeed")
                .expect("batch enrich should have created beatport cache entry")
        };
        let batch_json: serde_json::Value = serde_json::from_str(
            batch_cache
                .response_json
                .as_deref()
                .expect("batch beatport cache should contain JSON"),
        )
        .expect("batch beatport cache JSON should parse");
        assert!(
            batch_json
                .get("genre")
                .and_then(serde_json::Value::as_str)
                .is_some(),
            "batch beatport cache should have string 'genre'"
        );
        assert!(
            batch_json.get("genres").is_none(),
            "beatport cache should not be transformed into discogs-style 'genres' schema"
        );

        let batch_fields: HashSet<String> = batch_json
            .as_object()
            .expect("batch beatport cache should be object")
            .keys()
            .cloned()
            .collect();
        assert_eq!(
            batch_fields, individual_fields,
            "batch and individual beatport cache JSON should share the same schema"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn resolve_tracks_data_batch_consistency() {
        let Some((server, _store_dir)) =
            create_real_server_with_temp_store(default_http_client_for_tests())
        else {
            eprintln!("Skipping: backup tarball not found (set REKORDBOX_TEST_BACKUP)");
            return;
        };

        let tracks = sample_real_tracks(&server, 5);
        assert!(
            !tracks.is_empty(),
            "integration test needs tracks from real DB"
        );
        let track_ids: Vec<String> = tracks.iter().map(|t| t.id.clone()).collect();

        let batch_result = server
            .resolve_tracks_data(Parameters(ResolveTracksDataParams {
                track_ids: Some(track_ids.clone()),
                playlist_id: None,
                query: None,
                artist: None,
                genre: None,
                has_genre: None,
                bpm_min: None,
                bpm_max: None,
                key: None,
                rating_min: None,
                max_tracks: Some(track_ids.len() as u32),
            }))
            .await
            .expect("batch resolve should succeed");
        let batch_payload = extract_json(&batch_result);
        let batch_items = batch_payload
            .as_array()
            .expect("batch resolve payload should be an array");
        assert_eq!(
            batch_items.len(),
            track_ids.len(),
            "batch resolve should return one entry per requested track"
        );

        let mut by_track_id: HashMap<String, serde_json::Value> = HashMap::new();
        for item in batch_items {
            let track_id = item
                .get("track_id")
                .and_then(serde_json::Value::as_str)
                .expect("resolved track item should include track_id");
            by_track_id.insert(track_id.to_string(), item.clone());
        }

        for track_id in &track_ids {
            let single_result = server
                .resolve_track_data(Parameters(ResolveTrackDataParams {
                    track_id: track_id.clone(),
                }))
                .await
                .expect("single resolve should succeed");
            let single_payload = extract_json(&single_result);
            assert_eq!(
                by_track_id
                    .get(track_id)
                    .expect("batch output should include every requested track"),
                &single_payload,
                "batch resolve output should match single-track resolve output"
            );
        }
    }

    #[test]
    fn golden_genres_fixture_is_well_formed() {
        let entries = load_golden_genres_fixture();
        assert!(
            !entries.is_empty(),
            "golden genres fixture should contain at least one entry"
        );

        let mut unique = HashSet::new();
        for entry in &entries {
            assert!(
                !entry.artist.trim().is_empty(),
                "fixture artist must be non-empty"
            );
            assert!(
                !entry.title.trim().is_empty(),
                "fixture title must be non-empty"
            );
            assert!(
                !entry.notes.trim().is_empty(),
                "fixture notes must be non-empty"
            );
            assert!(
                genre::is_known_genre(&entry.expected_genre),
                "expected_genre '{}' must be in taxonomy",
                entry.expected_genre
            );
            assert!(
                genre::normalize_genre(&entry.expected_genre).is_none(),
                "expected_genre '{}' must be canonical, not alias",
                entry.expected_genre
            );

            let key = format!(
                "{}::{}",
                entry.artist.to_lowercase(),
                entry.title.to_lowercase()
            );
            assert!(unique.insert(key), "duplicate (artist, title) in fixture");
        }
    }

    #[test]
    #[ignore]
    fn golden_dataset_genre_accuracy() {
        let entries = load_golden_genres_fixture();
        let Some(conn) = db::open_real_db() else {
            eprintln!("Skipping: backup tarball not found (set REKORDBOX_TEST_BACKUP)");
            return;
        };

        let mut compared = 0usize;
        let mut correct = 0usize;
        let mut missing_tracks = Vec::new();
        let mut no_genre = Vec::new();
        let mut mismatches = Vec::new();

        for entry in &entries {
            let Some(track) = find_track_by_artist_and_title(&conn, &entry.artist, &entry.title)
            else {
                missing_tracks.push(format!("{} - {}", entry.artist, entry.title));
                continue;
            };

            if track.genre.trim().is_empty() {
                no_genre.push(format!("{} - {}", entry.artist, entry.title));
                continue;
            }

            compared += 1;
            let actual = canonical_genre_name(&track.genre);
            if actual.eq_ignore_ascii_case(&entry.expected_genre) {
                correct += 1;
            } else {
                mismatches.push(format!(
                    "{} - {}: expected '{}', actual '{}' ({})",
                    entry.artist, entry.title, entry.expected_genre, actual, entry.notes
                ));
            }
        }

        let accuracy = if compared == 0 {
            0.0
        } else {
            (correct as f64 / compared as f64) * 100.0
        };
        eprintln!(
            "[integration] golden dataset: total={} compared={} correct={} accuracy={:.1}%",
            entries.len(),
            compared,
            correct,
            accuracy
        );
        if !missing_tracks.is_empty() {
            eprintln!("[integration] missing tracks ({}):", missing_tracks.len());
            for item in &missing_tracks {
                eprintln!("  - {item}");
            }
        }
        if !no_genre.is_empty() {
            eprintln!(
                "[integration] tracks with empty genre ({}):",
                no_genre.len()
            );
            for item in &no_genre {
                eprintln!("  - {item}");
            }
        }
        if !mismatches.is_empty() {
            eprintln!("[integration] mismatches ({}):", mismatches.len());
            for item in &mismatches {
                eprintln!("  - {item}");
            }
        }

        assert!(
            !missing_tracks.is_empty() || compared > 0,
            "fixture should either report missing tracks or compare at least one track"
        );
    }

    // --- resolve_single_track unit tests ---

    fn make_test_track(id: &str, genre: &str, bpm: f64, key: &str) -> crate::types::Track {
        crate::types::Track {
            id: id.to_string(),
            title: format!("Track {id}"),
            artist: "Test Artist".to_string(),
            album: "Test Album".to_string(),
            genre: genre.to_string(),
            bpm,
            key: key.to_string(),
            rating: 3,
            comments: "test comment".to_string(),
            color: "Rose".to_string(),
            color_code: 1,
            label: "Test Label".to_string(),
            remixer: "".to_string(),
            year: 2023,
            length: 300,
            file_path: "/music/test.flac".to_string(),
            play_count: 5,
            bit_rate: 1411,
            sample_rate: 44100,
            file_type: 5,
            date_added: "2023-01-15".to_string(),
        }
    }

    #[test]
    fn resolve_single_track_rekordbox_only() {
        let track = make_test_track("t1", "Deep House", 126.0, "Am");
        let result = resolve_single_track(&track, None, None, None, None, false, None);

        // Verify rekordbox section present
        let rb = result
            .get("rekordbox")
            .expect("rekordbox section should exist");
        assert_eq!(rb["title"], "Track t1");
        assert_eq!(rb["artist"], "Test Artist");
        assert_eq!(rb["genre"], "Deep House");
        assert_eq!(rb["bpm"], 126.0);
        assert_eq!(rb["key"], "Am");
        assert_eq!(rb["duration_s"], 300);
        assert_eq!(rb["year"], 2023);
        assert_eq!(rb["rating"], 3);
        assert_eq!(rb["label"], "Test Label");

        // Null sections when no cache
        assert!(
            result["audio_analysis"].is_null(),
            "audio_analysis should be null without cache"
        );
        assert!(
            result["discogs"].is_null(),
            "discogs should be null without cache"
        );
        assert!(
            result["beatport"].is_null(),
            "beatport should be null without cache"
        );
        assert!(
            result["staged_changes"].is_null(),
            "staged_changes should be null without staged"
        );

        // Data completeness
        let dc = result
            .get("data_completeness")
            .expect("data_completeness should exist");
        assert_eq!(dc["rekordbox"], true);
        assert_eq!(dc["stratum_dsp"], false);
        assert_eq!(dc["essentia"], false);
        assert_eq!(dc["essentia_installed"], false);
        assert_eq!(dc["discogs"], false);
        assert_eq!(dc["beatport"], false);

        // Genre taxonomy — "Deep House" is canonical
        let gt = result
            .get("genre_taxonomy")
            .expect("genre_taxonomy should exist");
        assert_eq!(gt["current_genre_canonical"], "Deep House");
    }

    #[test]
    fn resolve_single_track_with_staged_changes() {
        let track = make_test_track("t2", "House", 128.0, "Cm");
        let staged = crate::types::TrackChange {
            track_id: "t2".to_string(),
            genre: Some("Deep House".to_string()),
            comments: None,
            rating: Some(5),
            color: None,
        };
        let result = resolve_single_track(&track, None, None, None, None, false, Some(&staged));

        let sc = result
            .get("staged_changes")
            .expect("staged_changes should exist");
        assert!(
            !sc.is_null(),
            "staged_changes should not be null when changes are staged"
        );
        assert_eq!(sc["genre"], "Deep House");
        assert!(sc["comments"].is_null(), "unstaged field should be null");
        assert_eq!(sc["rating"], 5);
        assert!(sc["color"].is_null(), "unstaged field should be null");
    }

    #[test]
    fn resolve_single_track_taxonomy_mappings() {
        // Track with an alias genre
        let track = make_test_track("t3", "Electronica", 130.0, "Fm");

        // Create mock Discogs enrichment with known, alias, and unknown styles
        let discogs_json = serde_json::json!({
            "title": "Some Release",
            "year": "2020",
            "label": "Some Label",
            "genres": ["Electronic"],
            "styles": ["Deep House", "Garage House", "Some Unknown Style"],
            "fuzzy_match": false,
        });
        let discogs_cache = store::CachedEnrichment {
            provider: "discogs".to_string(),
            query_artist: "test artist".to_string(),
            query_title: "track t3".to_string(),
            match_quality: Some("exact".to_string()),
            response_json: Some(serde_json::to_string(&discogs_json).unwrap()),
            created_at: "2024-01-01".to_string(),
        };

        // Create mock Beatport enrichment with a known genre
        let beatport_json = serde_json::json!({
            "genre": "Techno",
            "bpm": 130,
            "key": "Fm",
            "track_name": "Track t3",
            "artists": ["Test Artist"],
        });
        let beatport_cache = store::CachedEnrichment {
            provider: "beatport".to_string(),
            query_artist: "test artist".to_string(),
            query_title: "track t3".to_string(),
            match_quality: Some("exact".to_string()),
            response_json: Some(serde_json::to_string(&beatport_json).unwrap()),
            created_at: "2024-01-01".to_string(),
        };

        let result = resolve_single_track(
            &track,
            Some(&discogs_cache),
            Some(&beatport_cache),
            None,
            None,
            false,
            None,
        );

        // Data completeness
        let dc = &result["data_completeness"];
        assert_eq!(dc["discogs"], true);
        assert_eq!(dc["beatport"], true);
        assert_eq!(dc["stratum_dsp"], false);

        // Genre taxonomy
        let gt = &result["genre_taxonomy"];

        // "Electronica" is an alias for "Techno"
        assert_eq!(gt["current_genre_canonical"], "Techno");

        // Discogs style mappings
        let dsm = gt["discogs_style_mappings"]
            .as_array()
            .expect("should be array");
        assert_eq!(dsm.len(), 3);

        // Deep House — exact match (canonical genre)
        let dh = dsm
            .iter()
            .find(|m| m["style"] == "Deep House")
            .expect("Deep House mapping");
        assert_eq!(dh["mapping_type"], "exact");
        assert_eq!(dh["maps_to"], "Deep House");

        // Garage House — alias mapping
        let gh = dsm
            .iter()
            .find(|m| m["style"] == "Garage House")
            .expect("Garage House mapping");
        // "Garage House" is not in the alias map, so it should be unknown
        // Let's check: normalize_genre("Garage House") — it's not in alias_map
        // Actually: "gospel house" -> "House" is in the alias map but not "garage house"
        // So Garage House should be unknown
        assert_eq!(gh["mapping_type"], "unknown");
        assert!(gh["maps_to"].is_null());

        // Some Unknown Style — unknown
        let unknown = dsm
            .iter()
            .find(|m| m["style"] == "Some Unknown Style")
            .expect("unknown mapping");
        assert_eq!(unknown["mapping_type"], "unknown");
        assert!(unknown["maps_to"].is_null());

        // Beatport genre mapping
        let bgm = &gt["beatport_genre_mapping"];
        assert_eq!(bgm["genre"], "Techno");
        assert_eq!(bgm["mapping_type"], "exact");
        assert_eq!(bgm["maps_to"], "Techno");

        // Enrichment data is present
        assert!(
            result["discogs"].is_object(),
            "discogs should be parsed object"
        );
        assert!(
            result["beatport"].is_object(),
            "beatport should be parsed object"
        );
    }

    #[test]
    fn resolve_single_track_empty_genre_is_null() {
        let track = make_test_track("t4", "", 0.0, "");
        let result = resolve_single_track(&track, None, None, None, None, false, None);

        let gt = &result["genre_taxonomy"];
        assert!(
            gt["current_genre_canonical"].is_null(),
            "empty genre should map to null canonical"
        );
    }

    #[test]
    fn resolve_single_track_unknown_genre_maps_to_null() {
        let track = make_test_track("t5", "Polka", 120.0, "C");
        let result = resolve_single_track(&track, None, None, None, None, false, None);

        let gt = &result["genre_taxonomy"];
        assert!(
            gt["current_genre_canonical"].is_null(),
            "unknown genre 'Polka' should map to null"
        );
    }

    #[test]
    fn resolve_single_track_with_stratum_agreement() {
        let track = make_test_track("t6", "Techno", 128.0, "Am");

        // Stratum cache with matching BPM and key
        let stratum_json = serde_json::json!({
            "bpm": 128.5,
            "key": "Am",
            "analyzer_version": "0.1.0",
        });
        let stratum_cache = store::CachedAudioAnalysis {
            file_path: "/music/test.flac".to_string(),
            analyzer: "stratum-dsp".to_string(),
            file_size: 12345,
            file_mtime: 1700000000,
            analysis_version: "0.1.0".to_string(),
            features_json: serde_json::to_string(&stratum_json).unwrap(),
            created_at: "2024-01-01".to_string(),
        };

        let result =
            resolve_single_track(&track, None, None, Some(&stratum_cache), None, false, None);

        let aa = result
            .get("audio_analysis")
            .expect("audio_analysis should exist");
        assert!(
            !aa.is_null(),
            "audio_analysis should not be null with stratum cache"
        );
        assert_eq!(
            aa["bpm_agreement"], true,
            "BPM 128.0 vs 128.5 should agree (within 2.0)"
        );
        assert_eq!(aa["key_agreement"], true, "Key Am vs Am should agree");
        assert!(
            aa["stratum_dsp"].is_object(),
            "stratum_dsp should be the parsed features"
        );
        assert!(
            aa["essentia"].is_null(),
            "essentia should be null when not cached"
        );

        let dc = &result["data_completeness"];
        assert_eq!(dc["stratum_dsp"], true);
    }

    #[test]
    fn resolve_single_track_with_essentia_cache() {
        let track = make_test_track("t6b", "Techno", 128.0, "Am");
        let essentia_json = serde_json::json!({
            "danceability": 0.82,
            "loudness_integrated": -8.4,
            "rhythm_regularity": 0.91,
            "analyzer_version": "2.1b6.dev1389"
        });
        let essentia_cache = store::CachedAudioAnalysis {
            file_path: "/music/test.flac".to_string(),
            analyzer: "essentia".to_string(),
            file_size: 12345,
            file_mtime: 1700000000,
            analysis_version: "2.1b6.dev1389".to_string(),
            features_json: serde_json::to_string(&essentia_json).unwrap(),
            created_at: "2024-01-01".to_string(),
        };

        let result =
            resolve_single_track(&track, None, None, None, Some(&essentia_cache), true, None);

        let aa = &result["audio_analysis"];
        assert!(
            aa.is_object(),
            "audio_analysis should be populated when essentia cache exists"
        );
        assert!(
            aa["stratum_dsp"].is_null(),
            "stratum_dsp should remain null when not cached"
        );
        assert!(
            aa["essentia"].is_object(),
            "essentia should expose cached analysis JSON"
        );
        assert_eq!(aa["essentia"]["danceability"], 0.82);

        let dc = &result["data_completeness"];
        assert_eq!(dc["essentia"], true);
        assert_eq!(dc["essentia_installed"], true);
    }

    #[test]
    fn resolve_single_track_stratum_disagreement() {
        let track = make_test_track("t7", "House", 128.0, "Am");

        let stratum_json = serde_json::json!({
            "bpm": 64.0,
            "key": "Cm",
            "analyzer_version": "0.1.0",
        });
        let stratum_cache = store::CachedAudioAnalysis {
            file_path: "/music/test.flac".to_string(),
            analyzer: "stratum-dsp".to_string(),
            file_size: 12345,
            file_mtime: 1700000000,
            analysis_version: "0.1.0".to_string(),
            features_json: serde_json::to_string(&stratum_json).unwrap(),
            created_at: "2024-01-01".to_string(),
        };

        let result =
            resolve_single_track(&track, None, None, Some(&stratum_cache), None, false, None);

        let aa = &result["audio_analysis"];
        assert_eq!(
            aa["bpm_agreement"], false,
            "BPM 128.0 vs 64.0 should disagree"
        );
        assert_eq!(aa["key_agreement"], false, "Key Am vs Cm should disagree");
    }

    #[test]
    fn resolve_single_track_enrichment_no_match_returns_null() {
        let track = make_test_track("t8", "House", 126.0, "Am");

        // Cache entry exists but response_json is None (no match)
        let discogs_cache = store::CachedEnrichment {
            provider: "discogs".to_string(),
            query_artist: "test artist".to_string(),
            query_title: "track t8".to_string(),
            match_quality: Some("none".to_string()),
            response_json: None,
            created_at: "2024-01-01".to_string(),
        };

        let result =
            resolve_single_track(&track, Some(&discogs_cache), None, None, None, false, None);

        // discogs cached but no match -> null enrichment data, but data_completeness = true
        assert!(
            result["discogs"].is_null(),
            "discogs with no response_json should be null"
        );
        assert_eq!(
            result["data_completeness"]["discogs"], true,
            "cache entry exists so completeness is true"
        );
    }
}
