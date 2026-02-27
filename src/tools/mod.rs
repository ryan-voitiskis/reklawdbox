use std::sync::{Arc, Mutex, OnceLock};

use rmcp::handler::server::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ServerCapabilities, ServerInfo};
use rmcp::{ErrorData as McpError, ServerHandler, tool, tool_handler, tool_router};
use rusqlite::Connection;

mod analysis;
mod audit_handlers;
mod audio_handlers;
mod audio_scan;
mod batch;
mod corpus_helpers;
mod discogs_auth;
mod enrich_handlers;
mod enrichment;
mod essentia;
mod file_tag_handlers;
mod library_handlers;
mod params;
mod resolve_handlers;
mod resolve;
mod scoring;
mod sequencing_handlers;
mod staging_handlers;

use analysis::*;
use audit_handlers::*;
use audio_handlers::*;
use audio_scan::*;
use batch::*;
use corpus_helpers::*;
use discogs_auth::*;
use enrich_handlers::*;
use enrichment::*;
use essentia::*;
pub(crate) use essentia::probe_essentia_python_path;
use file_tag_handlers::*;
use library_handlers::*;
use params::*;
use resolve_handlers::*;
use resolve::*;
use scoring::*;
use sequencing_handlers::*;
use staging_handlers::*;

use crate::changes::ChangeManager;
use crate::db;
use crate::discogs;
use crate::store;

pub(super) fn mcp_internal_error(msg: String) -> McpError {
    McpError::internal_error(msg, None)
}

/// Inner shared state (not Clone).
pub(super) struct ServerState {
    pub(super) db: OnceLock<Result<Mutex<Connection>, String>>,
    pub(super) internal_db: OnceLock<Result<Mutex<Connection>, String>>,
    pub(super) essentia_python: OnceLock<Option<String>>,
    pub(super) essentia_python_override: Mutex<Option<String>>,
    pub(super) essentia_setup_lock: tokio::sync::Mutex<()>,
    pub(super) discogs_pending: Mutex<Option<discogs::PendingDeviceSession>>,
    pub(super) db_path: Option<String>,
    pub(super) changes: ChangeManager,
    pub(super) http: reqwest::Client,
}

#[derive(Clone)]
pub struct ReklawdboxServer {
    state: Arc<ServerState>,
    tool_router: ToolRouter<Self>,
}

impl ReklawdboxServer {
    pub(super) fn rekordbox_conn(&self) -> Result<std::sync::MutexGuard<'_, Connection>, McpError> {
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

    pub(super) fn cache_store_conn(&self) -> Result<std::sync::MutexGuard<'_, Connection>, McpError> {
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

    pub(super) fn cache_store_path(&self) -> String {
        std::env::var("CRATE_DIG_STORE_PATH")
            .unwrap_or_else(|_| store::default_path().to_string_lossy().to_string())
    }

    pub(super) fn essentia_python_path(&self) -> Option<String> {
        if let Ok(guard) = self.state.essentia_python_override.lock()
            && let Some(ref path) = *guard
        {
            return Some(path.clone());
        }
        self.state
            .essentia_python
            .get_or_init(probe_essentia_python_path)
            .clone()
    }
}

#[tool_router]
impl ReklawdboxServer {
    pub fn new(db_path: Option<String>) -> Self {
        let http = reqwest::Client::builder()
            .user_agent("Reklawdbox/0.1")
            .build()
            .expect("failed to build HTTP client");
        Self {
            state: Arc::new(ServerState {
                db: OnceLock::new(),
                internal_db: OnceLock::new(),
                essentia_python: OnceLock::new(),
                essentia_python_override: Mutex::new(None),
                essentia_setup_lock: tokio::sync::Mutex::new(()),
                discogs_pending: Mutex::new(None),
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
        handle_search_tracks(self.rekordbox_conn()?, params.0)
    }

    #[tool(description = "Get full details for a specific track by ID")]
    async fn get_track(
        &self,
        params: Parameters<GetTrackParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_get_track(self.rekordbox_conn()?, params.0)
    }

    #[tool(description = "List all playlists with track counts")]
    async fn get_playlists(&self) -> Result<CallToolResult, McpError> {
        handle_get_playlists(self.rekordbox_conn()?)
    }

    #[tool(description = "List tracks in a specific playlist")]
    async fn get_playlist_tracks(
        &self,
        params: Parameters<GetPlaylistTracksParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_get_playlist_tracks(self.rekordbox_conn()?, params.0)
    }

    #[tool(name = "read_library", description = "Get library summary: track count, genre distribution, stats")]
    async fn get_library_summary(&self) -> Result<CallToolResult, McpError> {
        handle_get_library_summary(self.rekordbox_conn()?)
    }

    #[tool(description = "Get the configured genre taxonomy")]
    async fn get_genre_taxonomy(&self) -> Result<CallToolResult, McpError> {
        handle_get_genre_taxonomy()
    }

    #[tool(
        description = "Stage changes to track metadata (genre, comments, rating, color). Changes are held in memory until write_xml is called."
    )]
    async fn update_tracks(
        &self,
        params: Parameters<UpdateTracksParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_update_tracks(&self.state.changes, params.0)
    }

    #[tool(
        description = "Analyze all genres in the library and suggest normalizations. Returns alias (known mapping), unknown (needs manual decision), and canonical (already correct) sections."
    )]
    async fn suggest_normalizations(
        &self,
        params: Parameters<SuggestNormalizationsParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_suggest_normalizations(self.rekordbox_conn()?, params.0)
    }

    #[tool(description = "Preview all staged changes, showing what will differ from current state")]
    async fn preview_changes(
        &self,
        params: Parameters<PreviewChangesParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_preview_changes(self, params.0)
    }

    #[tool(
        description = "Write staged changes and optional playlists to a Rekordbox-compatible XML file. Runs backup first."
    )]
    async fn write_xml(
        &self,
        params: Parameters<WriteXmlParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_write_xml(self, params.0).await
    }

    #[tool(description = "Clear staged changes for specific tracks or all")]
    async fn clear_changes(
        &self,
        params: Parameters<ClearChangesParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_clear_changes(&self.state.changes, params.0)
    }

    #[tool(
        description = "Look up a track on Discogs for genre/style enrichment. Returns an object payload with lookup data plus cache metadata (`cache_hit`, optional `cached_at`). On no match, `result` is null. Results are cached. Pass track_id to auto-fill artist/title/album from the library."
    )]
    async fn lookup_discogs(
        &self,
        params: Parameters<LookupDiscogsParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_lookup_discogs(self, params.0).await
    }

    #[tool(
        description = "Look up a track on Beatport for genre/BPM/key enrichment. Returns an object payload with lookup data plus cache metadata (`cache_hit`, optional `cached_at`). On no match, `result` is null. Results are cached. Pass track_id to auto-fill artist/title from the library."
    )]
    async fn lookup_beatport(
        &self,
        params: Parameters<LookupBeatportParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_lookup_beatport(self, params.0).await
    }

    #[tool(
        description = "Batch enrich tracks via Discogs/Beatport. Select tracks by IDs, playlist, or search filters. Results are cached."
    )]
    async fn enrich_tracks(
        &self,
        params: Parameters<EnrichTracksParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_enrich_tracks(self, params.0).await
    }

    #[tool(
        description = "Analyze a single track's audio file with stratum-dsp and Essentia (when installed). Returns BPM, key, rhythm/loudness descriptors, and confidence scores. Results are cached."
    )]
    async fn analyze_track_audio(
        &self,
        params: Parameters<AnalyzeTrackAudioParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_analyze_track_audio(self, params.0).await
    }

    #[tool(
        description = "Batch analyze audio files with stratum-dsp and Essentia (when installed). Select tracks by IDs, playlist, or search filters. Results are cached."
    )]
    async fn analyze_audio_batch(
        &self,
        params: Parameters<AnalyzeAudioBatchParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_analyze_audio_batch(self, params.0).await
    }

    #[tool(
        description = "Install Essentia into a managed Python venv. Call this when analyze_track_audio reports essentia_available: false. Creates a venv, installs essentia via pip, and makes it available immediately (no restart needed)."
    )]
    async fn setup_essentia(&self) -> Result<CallToolResult, McpError> {
        handle_setup_essentia(self).await
    }

    #[tool(
        description = "Score a single transition between two tracks using key, BPM, energy, genre, brightness, and rhythm compatibility."
    )]
    async fn score_transition(
        &self,
        params: Parameters<ScoreTransitionParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_score_transition(self, params.0)
    }

    #[tool(
        description = "Rank pool tracks as transition candidates from a reference track. Scores each candidate using key, BPM, energy, genre, brightness, and rhythm compatibility. Optionally target a specific BPM for trajectory-aware scoring."
    )]
    async fn query_transition_candidates(
        &self,
        params: Parameters<QueryTransitionCandidatesParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_query_transition_candidates(self, params.0)
    }

    #[tool(
        description = "Generate candidate set orderings from a track pool using beam search sequencing. Use beam_width to control search breadth (1=greedy, higher=more candidates). Use bpm_range for BPM trajectory planning."
    )]
    async fn build_set(
        &self,
        params: Parameters<BuildSetParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_build_set(self, params.0)
    }

    #[tool(
        description = "Get all available data for a track in one call: Rekordbox metadata, cached audio analysis, cached enrichment, staged changes, and genre taxonomy mappings. Cache-only — never triggers external calls."
    )]
    async fn resolve_track_data(
        &self,
        params: Parameters<ResolveTrackDataParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_resolve_track_data(self, params.0)
    }

    #[tool(
        description = "Get all available data for multiple tracks. Same as resolve_track_data but batched. Cache-only — never triggers external calls."
    )]
    async fn resolve_tracks_data(
        &self,
        params: Parameters<ResolveTracksDataParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_resolve_tracks_data(self, params.0)
    }

    #[tool(
        description = "Report cache completeness for a filtered track scope. Cache-only — no external calls."
    )]
    async fn cache_coverage(
        &self,
        params: Parameters<ResolveTracksDataParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_cache_coverage(self, params.0)
    }

    // -----------------------------------------------------------------------
    // Native file-tag tools
    // -----------------------------------------------------------------------

    #[tool(
        description = "Read metadata tags directly from audio files on disk. Supports FLAC, MP3, WAV, M4A, AAC, AIFF. Provide exactly one input selector: paths, track_ids, or directory."
    )]
    async fn read_file_tags(
        &self,
        params: Parameters<ReadFileTagsParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_read_file_tags(self, params.0).await
    }

    #[tool(
        description = "Write metadata tags to audio files on disk. Supports setting and deleting individual fields. Use dry_run to preview changes before writing."
    )]
    async fn write_file_tags(
        &self,
        params: Parameters<WriteFileTagsParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_write_file_tags(params.0).await
    }

    #[tool(
        description = "Extract cover art from an audio file and save to disk."
    )]
    async fn extract_cover_art(
        &self,
        params: Parameters<ExtractCoverArtParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_extract_cover_art(params.0).await
    }

    #[tool(
        description = "Embed cover art into one or more audio files."
    )]
    async fn embed_cover_art(
        &self,
        params: Parameters<EmbedCoverArtParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_embed_cover_art(params.0).await
    }

    // -----------------------------------------------------------------------
    // Audit engine
    // -----------------------------------------------------------------------

    #[tool(
        description = "Collection audit engine. Scan files for convention violations, query/resolve issues, and get summaries. Operations: scan, query_issues, resolve_issues, get_summary."
    )]
    async fn audit_state(
        &self,
        params: Parameters<AuditOperation>,
    ) -> Result<CallToolResult, McpError> {
        handle_audit_state(self.cache_store_path(), params.0).await
    }
}

#[tool_handler]
impl ServerHandler for ReklawdboxServer {
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
mod tests;
