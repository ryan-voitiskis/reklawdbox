use std::sync::{Arc, Mutex, OnceLock};

use rmcp::handler::server::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ServerCapabilities, ServerInfo};
use rmcp::{ErrorData as McpError, ServerHandler, tool, tool_handler, tool_router};
use rusqlite::Connection;

mod album_handlers;
mod analysis;
mod audio_handlers;
mod audio_scan;
mod audit_handlers;
mod batch;
mod classify_handler;
mod discogs_auth;
mod enrich_handlers;
mod enrichment;
mod enrichment_cache;
pub(crate) mod essentia;
mod file_tag_handlers;
mod health_handlers;
mod help_handler;
mod history_handlers;
mod label_handlers;
mod library_handlers;
mod params;
mod pool_handlers;
mod resolve;
mod resolve_handlers;
mod scoring;
mod sequencing_handlers;
mod staging_handlers;
mod weight_preset_handlers;
mod weight_resolve;
mod year_handlers;

use album_handlers::*;
use analysis::*;
use audio_handlers::*;
use audio_scan::*;
use audit_handlers::*;
use batch::*;
use classify_handler::*;
use discogs_auth::*;
use enrich_handlers::*;
use enrichment::*;
pub(crate) use essentia::probe_essentia_python_path;
use essentia::*;
use file_tag_handlers::*;
use health_handlers::*;
use help_handler::*;
use history_handlers::*;
use label_handlers::*;
use library_handlers::*;
use params::*;
use pool_handlers::*;
use resolve::*;
use resolve_handlers::*;
use scoring::*;
use sequencing_handlers::*;
use staging_handlers::*;
use weight_preset_handlers::*;
use weight_resolve::*;
use year_handlers::*;

use crate::changes::ChangeManager;
use crate::db;
use crate::discogs;
use crate::store;

pub(super) fn mcp_internal_error(msg: String) -> McpError {
    McpError::internal_error(msg, None)
}

pub(super) fn db_error(e: rusqlite::Error) -> McpError {
    McpError::internal_error(format!("DB error: {e}"), None)
}

pub(super) fn cache_error(e: rusqlite::Error) -> McpError {
    McpError::internal_error(format!("Cache error: {e}"), None)
}

pub(super) struct ServerState {
    pub(super) db: OnceLock<Result<Mutex<Connection>, String>>,
    pub(super) internal_db: OnceLock<Result<Mutex<Connection>, String>>,
    pub(super) essentia_python: OnceLock<Option<String>>,
    pub(super) essentia_python_override: Mutex<Option<String>>,
    pub(super) essentia_setup_lock: tokio::sync::Mutex<()>,
    pub(super) discogs_pending: Mutex<Option<discogs::PendingDeviceSession>>,
    pub(super) db_path: Option<String>,
    /// Explicit store path override (used by tests and batch tasks that need
    /// to open separate read-only / writer connections to the same file).
    pub(super) store_path: Option<String>,
    pub(super) changes: ChangeManager,
    pub(super) http: reqwest::Client,
    /// Set by backfill_labels when unlabeled tracks remain. Stores the count.
    /// write_xml checks this and refuses to export unless the agent explicitly
    /// acknowledges that label research is complete (skip_label_gate=true).
    pub(super) label_research_gate: std::sync::atomic::AtomicU32,
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

    pub(super) fn cache_store_conn(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, Connection>, McpError> {
        let result = self.state.internal_db.get_or_init(|| {
            let path = match self.state.store_path {
                Some(ref p) => std::path::PathBuf::from(p),
                None => std::env::var("CRATE_DIG_STORE_PATH")
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(|_| store::default_path()),
            };
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
        if let Some(ref p) = self.state.store_path {
            return p.clone();
        }
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
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(30))
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
                store_path: None,
                changes: ChangeManager::new(),
                http,
                label_research_gate: std::sync::atomic::AtomicU32::new(0),
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

    #[tool(
        name = "read_library",
        description = "Get library summary: track count, genre distribution, stats"
    )]
    async fn get_library_summary(&self) -> Result<CallToolResult, McpError> {
        handle_get_library_summary(self.rekordbox_conn()?)
    }

    #[tool(description = "Get the configured genre taxonomy")]
    async fn get_genre_taxonomy(&self) -> Result<CallToolResult, McpError> {
        handle_get_genre_taxonomy()
    }

    #[tool(description = "List recent DJ sessions from Rekordbox play history")]
    async fn get_sessions(
        &self,
        params: Parameters<GetSessionsParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_get_sessions(self.rekordbox_conn()?, params.0)
    }

    #[tool(description = "Get the ordered track list for a specific DJ session")]
    async fn get_session_tracks(
        &self,
        params: Parameters<GetSessionTracksParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_get_session_tracks(self.rekordbox_conn()?, params.0)
    }

    #[tool(
        description = "Get per-track play statistics scoped by search filters. Shows play counts, last played dates, and session appearances. Use include_unplayed to find tracks that have never been played."
    )]
    async fn get_play_stats(
        &self,
        params: Parameters<GetPlayStatsParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_get_play_stats(self.rekordbox_conn()?, params.0)
    }

    #[tool(
        description = "Get step-by-step workflow SOPs. Pass a topic for the full SOP, omit for the menu."
    )]
    async fn help(&self, params: Parameters<HelpParams>) -> Result<CallToolResult, McpError> {
        handle_help(params.0)
    }

    #[tool(
        description = "Stage changes to track metadata (genre, comments, rating, color, label, year, album). Changes are held in memory until write_xml is called."
    )]
    async fn update_tracks(
        &self,
        params: Parameters<UpdateTracksParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_update_tracks(&self.state.changes, params.0)
    }

    #[tool(
        description = "Analyze all genres in the library and suggest normalizations. Returns alias mappings grouped by genre (e.g. 'Hip-Hop' → 'Hip Hop', 5 tracks), unknown genres needing manual classification, and canonical genre counts. Use stage_aliases=true to auto-stage all alias normalizations in one shot."
    )]
    async fn suggest_normalizations(
        &self,
        params: Parameters<SuggestNormalizationsParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_suggest_normalizations(self.rekordbox_conn()?, &self.state.changes, params.0)
    }

    #[tool(
        description = "Preview all staged changes, showing what will differ from current state. Use format=\"summary\" to get aggregate counts by field and genre instead of full per-track diffs."
    )]
    async fn preview_changes(
        &self,
        params: Parameters<PreviewChangesParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_preview_changes(self, params.0)
    }

    #[tool(
        description = "Write staged changes and optional playlists to a Rekordbox-compatible XML file. Runs backup first. If backfill_labels was run and found unlabeled tracks, this tool will refuse to export until label research is complete (step 3 of metadata backfill SOP). Pass skip_label_gate=true only after completing label research."
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
        description = "Auto-fill empty labels from enrichment caches (Discogs, MusicBrainz, Bandcamp, Beatport). Stages non-conflicting labels; reports conflicts (capped at max_conflicts, default 50) where Rekordbox and enrichment disagree. Set auto_enrich=true to automatically fetch Bandcamp data for uncached tracks before backfilling. Use preview_changes then write_xml to export."
    )]
    async fn backfill_labels(
        &self,
        params: Parameters<BackfillLabelsParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_backfill_labels(self, params.0).await
    }

    #[tool(
        description = "Auto-fill missing years (year=0) from file tags, folder paths, and enrichment cache (Discogs/Beatport/MusicBrainz/Bandcamp). Stages non-conflicting years; reports conflicts where Rekordbox and Discogs disagree. Set auto_enrich=true to automatically fetch Bandcamp and MusicBrainz data for uncached year-zero tracks before re-scanning. Use preview_changes then write_xml to export."
    )]
    async fn backfill_years(
        &self,
        params: Parameters<BackfillYearsParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_backfill_years(self, params.0).await
    }

    #[tool(
        description = "Auto-fill empty album names from file tags, folder paths, and enrichment cache (Bandcamp/Discogs). Only fills tracks with no album set. Skips noise (album = track title or artist name). Set auto_enrich=true to fetch Bandcamp data for uncached tracks first. Use preview_changes then write_xml to export."
    )]
    async fn backfill_albums(
        &self,
        params: Parameters<BackfillAlbumsParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_backfill_albums(self, params.0).await
    }

    #[tool(
        description = "Clear all caches (enrichment, audio analysis, audit state) and staged changes. Preserves Discogs broker session. Use this to reset to a clean slate before a fresh test run."
    )]
    async fn clear_caches(&self) -> Result<CallToolResult, McpError> {
        handle_clear_caches(self)
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
        description = "Look up a track on MusicBrainz for year/label data. Returns year from first-release-date and label from the best matching release. Results are cached. Pass track_id to auto-fill artist/title from the library."
    )]
    async fn lookup_musicbrainz(
        &self,
        params: Parameters<LookupMusicBrainzParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_lookup_musicbrainz(self, params.0).await
    }

    #[tool(
        description = "Look up a track on Bandcamp for year/label/tags data. Searches Bandcamp, then fetches the detail page for structured metadata. Particularly effective for underground/independent electronic music. Results are cached. Pass track_id to auto-fill artist/title from the library."
    )]
    async fn lookup_bandcamp(
        &self,
        params: Parameters<LookupBandcampParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_lookup_bandcamp(self, params.0).await
    }

    #[tool(
        description = "Batch enrich tracks via Discogs/Beatport/Bandcamp. Select tracks by IDs, playlist, or search filters. Results are cached."
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
        description = "Score pool compatibility between tracks. Three modes: pairwise (track_a + track_b), one-vs-pool (track_id + pool_track_ids), or cohesion (pool_track_ids only). Symmetric kernel — score(A,B) == score(B,A). Uses BPM, energy, key, genre, timbral, brightness, rhythm axes."
    )]
    async fn score_pool_compatibility(
        &self,
        params: Parameters<ScorePoolCompatibilityParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_score_pool_compatibility(self, params.0)
    }

    #[tool(
        description = "Expand a track pool by finding compatible additions from the library. Iterative greedy expansion — each addition is guaranteed compatible with the full pool. Returns additions with rationale and pool cohesion stats."
    )]
    async fn expand_pool(
        &self,
        params: Parameters<ExpandPoolParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_expand_pool(self, params.0)
    }

    #[tool(
        description = "Analyze a pool's internal compatibility, coverage, and structure. Reports cohesion, medoid, weak members, energy/BPM/key stats, and (when master_tempo=false) optimal reference BPM."
    )]
    async fn describe_pool(
        &self,
        params: Parameters<DescribePoolParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_describe_pool(self, params.0)
    }

    #[tool(
        description = "Discover natural track pools in a library subset. Uses Bron-Kerbosch clique enumeration on a thresholded compatibility graph. Returns overlapping pools with core/edge members and bridge tracks. Adjust threshold (0.3-0.95) to control pool tightness."
    )]
    async fn discover_pools(
        &self,
        params: Parameters<DiscoverPoolsParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_discover_pools(self, params.0)
    }

    #[tool(
        description = "Save a custom weight preset for reuse across sessions. Weights are auto-renormalized to sum to 1.0. Use list_weight_presets to see available presets."
    )]
    async fn save_weight_preset(
        &self,
        params: Parameters<SaveWeightPresetParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_save_weight_preset(self, params.0)
    }

    #[tool(
        description = "List available weight presets (both built-in and custom saved). Shows weights for each preset."
    )]
    async fn list_weight_presets(
        &self,
        params: Parameters<ListWeightPresetsParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_list_weight_presets(self, params.0)
    }

    #[tool(description = "Delete a custom saved weight preset. Cannot delete built-in presets.")]
    async fn delete_weight_preset(
        &self,
        params: Parameters<DeleteWeightPresetParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_delete_weight_preset(self, params.0)
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

    #[tool(
        description = "Apply genre decision tree to ungenred tracks. Returns genre recommendations with confidence levels (high/medium/low/insufficient), evidence strings, and ranked candidates. High/medium results are ready for approval; low/insufficient may benefit from agent review using artist/title context. Use format=\"compact\" when classifying all tracks upfront (returns only track_id, artist, title, genre, confidence, action). Use format=\"summary\" to get only confidence distribution and genre-grouped counts without per-track results. Use format=\"dispatch\" to get low/insufficient tracks grouped by artist for subagent dispatch. Follow up with resolve_tracks_data(format=\"classification\") for full evidence on tracks that need review. When track_ids are provided explicitly, tracks with existing (non-canonical) genres can also be classified. Use auto_stage=[\"high\",\"medium\"] to directly stage results at specified confidence levels, eliminating the need for a separate update_tracks call. Cache-only — never triggers external calls."
    )]
    async fn classify_tracks(
        &self,
        params: Parameters<ClassifyTracksParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_classify_tracks(self, params.0)
    }

    #[tool(
        description = "Verify existing genre tags against enrichment and audio evidence. Returns only conflicts (genre disagrees with evidence) and manual-review tracks. Confirmed tracks are silently counted in the summary. Cache-only — never triggers external calls."
    )]
    async fn audit_genres(
        &self,
        params: Parameters<AuditGenresParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_audit_genres(self, params.0)
    }

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

    #[tool(description = "Extract cover art from an audio file and save to disk.")]
    async fn extract_cover_art(
        &self,
        params: Parameters<ExtractCoverArtParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_extract_cover_art(params.0).await
    }

    #[tool(description = "Embed cover art into one or more audio files.")]
    async fn embed_cover_art(
        &self,
        params: Parameters<EmbedCoverArtParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_embed_cover_art(params.0).await
    }

    #[tool(
        description = "Collection audit engine. Scan files for convention violations, query/resolve issues, and get summaries. Operations: scan, query_issues, resolve_issues, get_summary."
    )]
    async fn audit_state(
        &self,
        params: Parameters<AuditOperation>,
    ) -> Result<CallToolResult, McpError> {
        let rb_db_path = self.state.db_path.clone().or_else(db::resolve_db_path);
        handle_audit_state(self.cache_store_path(), rb_db_path, params.0).await
    }

    #[tool(
        description = "Scan for tracks with missing audio files on disk. Optionally suggests relocated files by matching filenames across content roots."
    )]
    async fn scan_broken_links(
        &self,
        params: Parameters<ScanBrokenLinksParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_scan_broken_links(self, params.0).await
    }

    #[tool(
        description = "Find audio files on disk not imported into Rekordbox. Compares filesystem contents against the database for each content root."
    )]
    async fn scan_orphan_files(
        &self,
        params: Parameters<ScanOrphanFilesParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_scan_orphan_files(self, params.0).await
    }

    #[tool(
        description = "Find tracks not assigned to any playlist. Supports all search filters for scoping."
    )]
    async fn scan_playlist_coverage(
        &self,
        params: Parameters<ScanPlaylistCoverageParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_scan_playlist_coverage(self.rekordbox_conn()?, params.0)
    }

    #[tool(
        description = "Detect duplicate tracks by metadata (artist+title) or exact file hash (SHA-256). Each group includes a suggested_keep recommendation based on audio quality."
    )]
    async fn scan_duplicates(
        &self,
        params: Parameters<ScanDuplicatesParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_scan_duplicates(self, params.0).await
    }
}

#[tool_handler]
impl ServerHandler for ReklawdboxServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "Rekordbox library management server. Read-only DB access, staged XML export.\n\
                 \n\
                 Before using any workflow tools, call help() to load the matching SOP. Each \
                 workflow has prerequisite steps and a specific tool sequence \u{2014} following the \
                 SOP produces significantly better results than ad-hoc tool use.\n\
                 \n\
                 Call read_library to see the collection.\n\
                 Call help() for workflow menu, help(topic) for the full SOP."
                    .into(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod eval_scoring;
#[cfg(test)]
mod tests;
