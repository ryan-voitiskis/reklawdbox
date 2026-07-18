use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, OnceLock, Weak};

use rmcp::handler::server::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ServerCapabilities, ServerInfo};
use rmcp::{ErrorData as McpError, ServerHandler, tool, tool_handler, tool_router};
use rusqlite::Connection;

use crate::adapters::audio::probe_essentia_python_path;

use super::analysis::{
    AnalyzeAudioBatchOutput, AnalyzeAudioBatchParams, AnalyzeTrackAudioParams, CacheCoverageParams,
    handle_analyze_audio_batch, handle_analyze_track_audio, handle_cache_coverage,
    handle_setup_essentia,
};
use super::audit::{
    AuditOperation, ScanBrokenLinksParams, ScanDuplicatesOutput, ScanDuplicatesParams,
    ScanOrphanFilesParams, ScanPlaylistCoverageParams, handle_audit_state,
    handle_scan_broken_links, handle_scan_duplicates, handle_scan_orphan_files,
    handle_scan_playlist_coverage,
};
use super::classification as classification_transport;
use super::classification::{
    AuditGenresParams, ClassifyTracksParams, handle_audit_genres, handle_calibrate_audio_profiles,
    handle_calibration_coverage, handle_classify_tracks,
};
use super::context::ServerContext;
use super::enrichment::{
    EnrichTracksOutput, EnrichTracksParams, LookupBandcampParams, LookupDiscogsParams,
    LookupMusicBrainzParams, ResolveTrackDataParams, ResolveTracksDataParams, handle_enrich_tracks,
    handle_lookup_bandcamp, handle_lookup_discogs, handle_lookup_musicbrainz,
    handle_resolve_track_data, handle_resolve_tracks_data,
};
use super::error::mcp_internal_error;
use super::files::{
    EmbedCoverArtParams, ExtractCoverArtParams, ReadFileTagsParams, WriteFileTagsParams,
    handle_embed_cover_art, handle_extract_cover_art, handle_read_file_tags,
    handle_write_file_tags,
};
use super::help::{HelpParams, handle_help};
use super::library::{
    GetPlayStatsParams, GetPlaylistTracksParams, GetSessionTracksParams, GetSessionsParams,
    GetTrackParams, SearchTracksParams, handle_get_genre_taxonomy, handle_get_library_summary,
    handle_get_play_stats, handle_get_playlist_tracks, handle_get_playlists,
    handle_get_session_tracks, handle_get_sessions, handle_get_track, handle_search_tracks,
};
use super::metadata::{
    BackfillAlbumsParams, BackfillLabelsOutput, BackfillLabelsParams, BackfillYearsParams,
    ClearChangesParams, PreviewChangesParams, SuggestNormalizationsParams, UpdateTracksParams,
    WriteXmlParams, handle_backfill_albums, handle_backfill_labels, handle_backfill_years,
    handle_clear_caches, handle_clear_changes, handle_preview_changes,
    handle_suggest_normalizations, handle_update_tracks, handle_write_xml,
};
use super::planning::{
    BuildSetParams, DeleteWeightPresetParams, DescribePoolParams, DiscoverPoolsParams,
    ExpandPoolParams, ListWeightPresetsParams, QueryTransitionCandidatesParams,
    SaveWeightPresetParams, ScorePoolCompatibilityParams, ScoreTransitionParams, handle_build_set,
    handle_delete_weight_preset, handle_describe_pool, handle_discover_pools, handle_expand_pool,
    handle_list_weight_presets, handle_query_transition_candidates, handle_save_weight_preset,
    handle_score_pool_compatibility, handle_score_transition,
};

use crate::adapters::rekordbox as db;
use crate::adapters::state as store;

#[derive(Clone)]
pub struct ReklawdboxServer {
    pub(super) context: Arc<ServerContext>,
    pub(super) tool_router: ToolRouter<Self>,
}

fn get_or_try_init_once<T, E>(
    cell: &OnceLock<T>,
    initialize: impl FnOnce() -> Result<T, E>,
) -> Result<&T, E> {
    if let Some(value) = cell.get() {
        return Ok(value);
    }

    match initialize() {
        Ok(candidate) => {
            let _ = cell.set(candidate);
            Ok(cell
                .get()
                .expect("successful once-cell initialization must publish a value"))
        }
        Err(error) => cell.get().ok_or(error),
    }
}

impl ReklawdboxServer {
    #[cfg(test)]
    pub(super) fn build_tool_router() -> ToolRouter<Self> {
        Self::tool_router()
    }

    pub(super) fn effective_db_path(&self) -> Result<PathBuf, McpError> {
        match get_or_try_init_once(&self.context.database.effective_db_path, || {
            let configured = self
                .context
                .database
                .db_path
                .clone()
                .or_else(db::resolve_db_path);
            resolve_effective_db_path(configured.as_deref())
        }) {
            Ok(path) => Ok(path.clone()),
            Err(message) => Err(mcp_internal_error(message)),
        }
    }

    pub(super) fn rekordbox_conn(&self) -> Result<std::sync::MutexGuard<'_, Connection>, McpError> {
        let mutex = get_or_try_init_once(&self.context.database.db, || {
            let path = self
                .effective_db_path()
                .map_err(|error| error.message.to_string())?;
            let path = path.to_str().ok_or_else(|| {
                "Effective Rekordbox database path is not valid UTF-8".to_string()
            })?;
            match db::open(path) {
                Ok(conn) => Ok::<_, String>(Mutex::new(conn)),
                Err(e) => Err(format!("Failed to open Rekordbox database: {e}")),
            }
        })
        .map_err(|message| McpError::internal_error(message, None))?;
        mutex
            .lock()
            .map_err(|_| McpError::internal_error("Database lock poisoned", None))
    }

    pub(super) fn cache_store_conn(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, Connection>, McpError> {
        let mutex = get_or_try_init_once(&self.context.database.internal_db, || {
            let path = match self.context.database.store_path {
                Some(ref p) => std::path::PathBuf::from(p),
                None => store::resolve_path(),
            };
            let path_str = path.to_string_lossy().to_string();
            match store::open(&path_str) {
                Ok(conn) => Ok::<_, String>(Mutex::new(conn)),
                Err(e) => Err(format!("Failed to open internal store: {e}")),
            }
        })
        .map_err(|message| McpError::internal_error(message, None))?;
        mutex
            .lock()
            .map_err(|_| McpError::internal_error("Internal store lock poisoned", None))
    }

    pub(super) fn cache_store_path(&self) -> String {
        if let Some(ref p) = self.context.database.store_path {
            return p.clone();
        }
        store::resolve_path().to_string_lossy().to_string()
    }

    pub(super) fn essentia_python_path(&self) -> Option<String> {
        if let Ok(guard) = self.context.analysis.essentia_python_override.lock()
            && let Some(ref path) = *guard
        {
            return Some(path.clone());
        }
        if self
            .context
            .analysis
            .essentia_probe_invalidated
            .load(Ordering::Acquire)
        {
            return None;
        }
        self.context
            .analysis
            .essentia_python
            .get_or_init(probe_essentia_python_path)
            .clone()
    }

    pub(super) fn activate_essentia_python_path(&self, python_path: String) -> Result<(), String> {
        let mut guard = self
            .context
            .analysis
            .essentia_python_override
            .lock()
            .map_err(|_| "essentia override lock poisoned".to_string())?;
        *guard = Some(python_path);
        self.context
            .analysis
            .essentia_probe_invalidated
            .store(false, Ordering::Release);
        Ok(())
    }

    pub(super) fn invalidate_essentia_python_path(&self) {
        if let Ok(mut guard) = self.context.analysis.essentia_python_override.lock() {
            *guard = None;
            self.context
                .analysis
                .essentia_probe_invalidated
                .store(true, Ordering::Release);
            return;
        }
        self.context
            .analysis
            .essentia_probe_invalidated
            .store(true, Ordering::Release);
    }

    pub(super) fn audio_file_mutation_lock(
        &self,
        canonical_path: &Path,
    ) -> Result<Arc<tokio::sync::Mutex<()>>, McpError> {
        let mut locks = self
            .context
            .mutation
            .audio_file_mutation_locks
            .lock()
            .map_err(|_| mcp_internal_error("Audio file mutation lock registry poisoned"))?;
        locks.retain(|_, lock| lock.strong_count() > 0);

        if let Some(lock) = locks.get(canonical_path).and_then(Weak::upgrade) {
            return Ok(lock);
        }

        let lock = Arc::new(tokio::sync::Mutex::new(()));
        locks.insert(canonical_path.to_path_buf(), Arc::downgrade(&lock));
        Ok(lock)
    }

    #[cfg(test)]
    pub(super) fn audio_file_mutation_registry_len(&self) -> Result<usize, McpError> {
        self.context
            .mutation
            .audio_file_mutation_locks
            .lock()
            .map(|locks| locks.len())
            .map_err(|_| mcp_internal_error("Audio file mutation lock registry poisoned"))
    }
}

#[tool_router(router = tool_router)]
impl ReklawdboxServer {
    pub fn new(db_path: Option<String>) -> Self {
        let http = reqwest::Client::builder()
            .user_agent("Reklawdbox/0.1")
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("failed to build HTTP client");
        Self {
            context: Arc::new(ServerContext::new(db_path, http)),
            tool_router: Self::tool_router(),
        }
    }

    #[tool(description = "Search and filter tracks in the Rekordbox library")]
    pub(super) async fn search_tracks(
        &self,
        params: Parameters<SearchTracksParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_search_tracks(self.rekordbox_conn()?, params.0)
    }

    #[tool(description = "Get full details for a specific track by ID")]
    pub(super) async fn get_track(
        &self,
        params: Parameters<GetTrackParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_get_track(self.rekordbox_conn()?, params.0)
    }

    #[tool(description = "List all playlists with track counts")]
    pub(super) async fn get_playlists(&self) -> Result<CallToolResult, McpError> {
        handle_get_playlists(self.rekordbox_conn()?)
    }

    #[tool(description = "List tracks in a specific playlist")]
    pub(super) async fn get_playlist_tracks(
        &self,
        params: Parameters<GetPlaylistTracksParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_get_playlist_tracks(self.rekordbox_conn()?, params.0)
    }

    #[tool(
        name = "read_library",
        description = "Get library summary: track count, genre distribution, stats"
    )]
    pub(super) async fn get_library_summary(&self) -> Result<CallToolResult, McpError> {
        handle_get_library_summary(self.rekordbox_conn()?)
    }

    #[tool(description = "Get the configured genre taxonomy")]
    pub(super) async fn get_genre_taxonomy(&self) -> Result<CallToolResult, McpError> {
        handle_get_genre_taxonomy()
    }

    #[tool(description = "List recent DJ sessions from Rekordbox play history")]
    pub(super) async fn get_sessions(
        &self,
        params: Parameters<GetSessionsParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_get_sessions(self.rekordbox_conn()?, params.0)
    }

    #[tool(description = "Get the ordered track list for a specific DJ session")]
    pub(super) async fn get_session_tracks(
        &self,
        params: Parameters<GetSessionTracksParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_get_session_tracks(self.rekordbox_conn()?, params.0)
    }

    #[tool(
        description = "Get per-track play statistics scoped by search filters. Shows play counts, last played dates, and session appearances. Use include_unplayed to find tracks that have never been played."
    )]
    pub(super) async fn get_play_stats(
        &self,
        params: Parameters<GetPlayStatsParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_get_play_stats(self.rekordbox_conn()?, params.0)
    }

    #[tool(
        description = "Get step-by-step workflow SOPs. Pass a topic for the full SOP, omit for the menu."
    )]
    pub(super) async fn help(
        &self,
        params: Parameters<HelpParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_help(params.0)
    }

    #[tool(
        description = "Stage changes to track metadata (genre, comments, rating, color, label, year, album). Changes are held in memory until write_xml is called."
    )]
    pub(super) async fn update_tracks(
        &self,
        params: Parameters<UpdateTracksParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_update_tracks(&self.context.mutation.changes, params.0)
    }

    #[tool(
        description = "Analyze all genres in the library and suggest normalizations. Returns alias mappings grouped by genre (e.g. 'Hip-Hop' → 'Hip Hop', 5 tracks), unknown genres needing manual classification, and canonical genre counts. Use stage_aliases=true to auto-stage all alias normalizations in one shot."
    )]
    pub(super) async fn suggest_normalizations(
        &self,
        params: Parameters<SuggestNormalizationsParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_suggest_normalizations(
            self.rekordbox_conn()?,
            &self.context.mutation.changes,
            params.0,
        )
    }

    #[tool(
        description = "Preview all staged changes, showing what will differ from current state. Use format=\"summary\" to get aggregate counts by field and genre instead of full per-track diffs."
    )]
    pub(super) async fn preview_changes(
        &self,
        params: Parameters<PreviewChangesParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_preview_changes(self, params.0)
    }

    #[tool(
        description = "Write staged changes and optional playlists to a Rekordbox-compatible XML file. Runs backup first. If backfill_labels was run and found unlabeled tracks, this tool will refuse to export until label research is complete (Step 1c of the metadata backfill SOP). Pass skip_label_gate=true only after completing label research."
    )]
    pub(super) async fn write_xml(
        &self,
        params: Parameters<WriteXmlParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_write_xml(self, params.0).await
    }

    #[tool(description = "Clear staged changes for specific tracks or all")]
    pub(super) async fn clear_changes(
        &self,
        params: Parameters<ClearChangesParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_clear_changes(&self.context.mutation.changes, params.0)
    }

    #[tool(
        description = "Auto-fill empty labels from enrichment caches using Discogs > MusicBrainz > Bandcamp precedence. Stages non-conflicting labels; pages conflicts with conflict_offset and conflict_page. Set auto_enrich=true to fetch missing MusicBrainz and Bandcamp keys before backfilling; provider counts report successful matches. Use dry_run=true and auto_enrich=false for later conflict pages. Use preview_changes then write_xml to export.",
        output_schema = rmcp::handler::server::tool::schema_for_output::<BackfillLabelsOutput>()
            .expect("backfill_labels output schema should be valid")
    )]
    pub(super) async fn backfill_labels(
        &self,
        params: Parameters<BackfillLabelsParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_backfill_labels(self, params.0).await
    }

    #[tool(
        description = "Auto-fill missing years (year=0) from file tags, folder paths, and enrichment cache (Discogs/MusicBrainz/Bandcamp). Stages non-conflicting years; reports conflicts where Rekordbox and Discogs disagree. Set auto_enrich=true to automatically fetch Bandcamp and MusicBrainz data for uncached year-zero tracks before re-scanning. Use preview_changes then write_xml to export."
    )]
    pub(super) async fn backfill_years(
        &self,
        params: Parameters<BackfillYearsParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_backfill_years(self, params.0).await
    }

    #[tool(
        description = "Auto-fill empty album names from file tags, folder paths, and enrichment cache (Bandcamp/Discogs). Only fills tracks with no album set. Skips noise (album = track title or artist name). Set auto_enrich=true to fetch Bandcamp data for uncached tracks first. Use preview_changes then write_xml to export."
    )]
    pub(super) async fn backfill_albums(
        &self,
        params: Parameters<BackfillAlbumsParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_backfill_albums(self, params.0).await
    }

    #[tool(
        description = "Clear all caches (enrichment, audio analysis, audit state) and staged changes. Preserves Discogs broker session. Use this to reset to a clean slate before a fresh test run."
    )]
    pub(super) async fn clear_caches(&self) -> Result<CallToolResult, McpError> {
        handle_clear_caches(self)
    }

    #[tool(
        description = "Look up a track on Discogs for genre/style enrichment. Returns an object payload with lookup data plus cache metadata (`cache_hit`, optional `cached_at`). On no match, `result` is null. Results are cached. Pass track_id to auto-fill artist/title/album from the library."
    )]
    pub(super) async fn lookup_discogs(
        &self,
        params: Parameters<LookupDiscogsParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_lookup_discogs(self, params.0).await
    }

    #[tool(
        description = "Look up a track on MusicBrainz for year/label data. Returns year from first-release-date and label from the best matching release. Results are cached. Pass track_id to auto-fill artist/title from the library."
    )]
    pub(super) async fn lookup_musicbrainz(
        &self,
        params: Parameters<LookupMusicBrainzParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_lookup_musicbrainz(self, params.0).await
    }

    #[tool(
        description = "Look up a track on Bandcamp for year/label/tags/cover data. Pass url with a direct Bandcamp /track/ or /album/ page to bypass Bandcamp search; album pages use the provided title to select the track. Particularly effective for underground/independent electronic music. Results are cached. Pass track_id to auto-fill artist/title from the library."
    )]
    pub(super) async fn lookup_bandcamp(
        &self,
        params: Parameters<LookupBandcampParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_lookup_bandcamp(self, params.0).await
    }

    #[tool(
        description = "Batch enrich pending tracks via Discogs/Bandcamp. Cached candidates do not consume max_tracks; continue with page.next_offset while page.has_more. Keep providers, skip_cached, and force_refresh fixed during traversal, or restart at offset 0. Results are cached.",
        output_schema = rmcp::handler::server::tool::schema_for_output::<EnrichTracksOutput>()
            .expect("enrich_tracks output schema should be valid")
    )]
    pub(super) async fn enrich_tracks(
        &self,
        params: Parameters<EnrichTracksParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_enrich_tracks(self, params.0).await
    }

    #[tool(
        description = "Analyze a single track's audio file with stratum-dsp and Essentia (when installed). Returns BPM, key, rhythm/loudness descriptors, and confidence scores. Results are cached."
    )]
    pub(super) async fn analyze_track_audio(
        &self,
        params: Parameters<AnalyzeTrackAudioParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_analyze_track_audio(self, params.0).await
    }

    #[tool(
        description = "Batch analyze pending audio files with stratum-dsp and Essentia (when installed). Current cached candidates do not consume max_tracks; continue with page.next_offset while page.has_more. If skip_cached or Essentia availability changes, restart at offset 0. Results are cached.",
        output_schema = rmcp::handler::server::tool::schema_for_output::<AnalyzeAudioBatchOutput>()
            .expect("analyze_audio_batch output schema should be valid")
    )]
    pub(super) async fn analyze_audio_batch(
        &self,
        params: Parameters<AnalyzeAudioBatchParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_analyze_audio_batch(self, params.0).await
    }

    #[tool(
        description = "Install or validate the supported CPython 3.14 Essentia runtime. With no valid expert override, uses the stable managed path and the pinned binary-only Essentia, NumPy, PyYAML, and six manifest. Activates immediately without a restart. Call this when analyze_track_audio reports essentia_available: false."
    )]
    pub(super) async fn setup_essentia(&self) -> Result<CallToolResult, McpError> {
        handle_setup_essentia(self).await
    }

    #[tool(
        description = "Score a single transition between two tracks using key, BPM, energy, genre, brightness, and rhythm compatibility."
    )]
    pub(super) async fn score_transition(
        &self,
        params: Parameters<ScoreTransitionParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_score_transition(self, params.0)
    }

    #[tool(
        description = "Rank pool tracks as transition candidates from a reference track. Scores each candidate using key, BPM, energy, genre, brightness, and rhythm compatibility. Optionally target a specific BPM for trajectory-aware scoring."
    )]
    pub(super) async fn query_transition_candidates(
        &self,
        params: Parameters<QueryTransitionCandidatesParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_query_transition_candidates(self, params.0)
    }

    #[tool(
        description = "Generate candidate set orderings from a track pool using beam search sequencing. Use beam_width to control search breadth (1=greedy, higher=more candidates). Use bpm_range for BPM trajectory planning."
    )]
    pub(super) async fn build_set(
        &self,
        params: Parameters<BuildSetParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_build_set(self, params.0)
    }

    #[tool(
        description = "Score pool compatibility between tracks. Three modes: pairwise (track_a + track_b), one-vs-pool (track_id + pool_track_ids), or cohesion (pool_track_ids only). Symmetric kernel — score(A,B) == score(B,A). Uses BPM, energy, key, genre, timbral, brightness, rhythm axes."
    )]
    pub(super) async fn score_pool_compatibility(
        &self,
        params: Parameters<ScorePoolCompatibilityParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_score_pool_compatibility(self, params.0)
    }

    #[tool(
        description = "Expand a track pool by finding compatible additions from the library. Iterative greedy expansion — each addition is guaranteed compatible with the full pool. Returns additions with rationale and pool cohesion stats."
    )]
    pub(super) async fn expand_pool(
        &self,
        params: Parameters<ExpandPoolParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_expand_pool(self, params.0)
    }

    #[tool(
        description = "Analyze a pool's internal compatibility, coverage, and structure. Reports cohesion, medoid, weak members, energy/BPM/key stats, and (when master_tempo=false) optimal reference BPM."
    )]
    pub(super) async fn describe_pool(
        &self,
        params: Parameters<DescribePoolParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_describe_pool(self, params.0)
    }

    #[tool(
        description = "Discover natural track pools in a library subset. Uses Bron-Kerbosch clique enumeration on a thresholded compatibility graph. Returns overlapping pools with core/edge members and bridge tracks. Adjust threshold (0.3-0.95) to control pool tightness."
    )]
    pub(super) async fn discover_pools(
        &self,
        params: Parameters<DiscoverPoolsParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_discover_pools(self, params.0)
    }

    #[tool(
        description = "Save a custom weight preset for reuse across sessions. Weights are auto-renormalized to sum to 1.0. Use list_weight_presets to see available presets."
    )]
    pub(super) async fn save_weight_preset(
        &self,
        params: Parameters<SaveWeightPresetParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_save_weight_preset(self, params.0)
    }

    #[tool(
        description = "List available weight presets (both built-in and custom saved). Shows weights for each preset."
    )]
    pub(super) async fn list_weight_presets(
        &self,
        params: Parameters<ListWeightPresetsParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_list_weight_presets(self, params.0)
    }

    #[tool(description = "Delete a custom saved weight preset. Cannot delete built-in presets.")]
    pub(super) async fn delete_weight_preset(
        &self,
        params: Parameters<DeleteWeightPresetParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_delete_weight_preset(self, params.0)
    }

    #[tool(
        description = "Get Rekordbox metadata plus cached Discogs enrichment, current Stratum and optional current Essentia analysis, staged changes, and genre taxonomy mappings for one track. Cache-only — never triggers external calls."
    )]
    pub(super) async fn resolve_track_data(
        &self,
        params: Parameters<ResolveTrackDataParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_resolve_track_data(self, params.0)
    }

    #[tool(
        description = "Batch Rekordbox metadata plus cached Discogs enrichment, current Stratum and optional current Essentia analysis, staged changes, and genre taxonomy mappings. Cache-only — never triggers external calls."
    )]
    pub(super) async fn resolve_tracks_data(
        &self,
        params: Parameters<ResolveTracksDataParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_resolve_tracks_data(self, params.0)
    }

    #[tool(
        description = "Report cache completeness for a filtered track scope, including Full/Degraded classification readiness from current Stratum and Essentia rows. Discogs readiness uses the same exact album-aware key and genre mapping as classification, distinguishing searched, matched, mapped, and usable rows. Cache-only — no external calls."
    )]
    pub(super) async fn cache_coverage(
        &self,
        params: Parameters<CacheCoverageParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_cache_coverage(self, params.0)
    }

    #[tool(
        description = "Apply the genre decision tree to ungenred tracks. Full mode requires fresh valid Stratum and Essentia rows. Missing, stale, or invalid analyzer evidence produces Degraded mode, caps confidence at Low, requires review, and never auto-stages. High confidence additionally requires agreement from at least two independent source groups (Discogs, a distinct Rekordbox label, or audio); Discogs styles and its fallback label count once. Current genre is only a low-confidence hint or unresolved-tie breaker. Use format=\"compact\", \"summary\", or \"dispatch\" for bounded workflows. Cache-only — never triggers external calls."
    )]
    pub(super) async fn classify_tracks(
        &self,
        params: Parameters<ClassifyTracksParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_classify_tracks(self, params.0)
    }

    #[tool(
        description = "Verify existing genre tags against enrichment and audio evidence using the same Full/Degraded readiness contract as classification. Degraded results require review. Strong Full high/medium confirmations are counted but omitted by default; low/insufficient confirmations remain visible because confidence determines review eligibility. Set include_confirmed=true to return every confirmation. Cache-only — never triggers external calls."
    )]
    pub(super) async fn audit_genres(
        &self,
        params: Parameters<AuditGenresParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_audit_genres(self, params.0)
    }

    #[tool(
        description = "Calibrate genre audio profiles from a playlist of verified tracks. Only rows with fresh valid Stratum and Essentia evidence plus scorable optional features can build candidates; partial or BPM-only rows cannot qualify or replace a usable registry. Stores profiles atomically with classifier/analyzer versions and a privacy-safe training fingerprint."
    )]
    pub(super) async fn calibrate_audio_profiles(
        &self,
        params: Parameters<classification_transport::CalibrateAudioProfilesParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_calibrate_audio_profiles(self, params.0)
    }

    #[tool(
        description = "Report per-genre verified-playlist calibration readiness. Distinguishes raw analyzer rows, complete Full-classification audio, missing or invalid Stratum/Essentia evidence, scorable optional features, candidate prototypes, and stored profile state: missing, fresh, training_changed, or incompatible. Read-only — never recalibrates."
    )]
    pub(super) async fn calibration_coverage(
        &self,
        params: Parameters<classification_transport::CalibrationCoverageParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_calibration_coverage(self, params.0)
    }

    #[tool(
        description = "Read metadata tags directly from audio files on disk. Supports FLAC, MP3, WAV, M4A, AAC, AIFF. Provide exactly one input selector: paths, track_ids, or directory."
    )]
    pub(super) async fn read_file_tags(
        &self,
        params: Parameters<ReadFileTagsParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_read_file_tags(self, params.0).await
    }

    #[tool(
        description = "Write metadata tags to audio files on disk. Supports setting and deleting individual fields. Use dry_run to preview changes before writing."
    )]
    pub(super) async fn write_file_tags(
        &self,
        params: Parameters<WriteFileTagsParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_write_file_tags(self, params.0).await
    }

    #[tool(description = "Extract cover art from an audio file and save to disk.")]
    pub(super) async fn extract_cover_art(
        &self,
        params: Parameters<ExtractCoverArtParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_extract_cover_art(params.0).await
    }

    #[tool(description = "Embed cover art into one or more audio files.")]
    pub(super) async fn embed_cover_art(
        &self,
        params: Parameters<EmbedCoverArtParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_embed_cover_art(self, params.0).await
    }

    #[tool(
        description = "Collection audit engine. Scan files for convention violations, query/resolve issues, and get summaries. Operations: scan, query_issues, resolve_issues, get_summary."
    )]
    pub(super) async fn audit_state(
        &self,
        params: Parameters<AuditOperation>,
    ) -> Result<CallToolResult, McpError> {
        let rb_db_path = self
            .context
            .database
            .db_path
            .clone()
            .or_else(db::resolve_db_path);
        handle_audit_state(self.cache_store_path(), rb_db_path, params.0).await
    }

    #[tool(
        description = "Scan for tracks with missing audio files on disk. Optionally suggests relocated files by matching filenames across content roots."
    )]
    pub(super) async fn scan_broken_links(
        &self,
        params: Parameters<ScanBrokenLinksParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_scan_broken_links(self, params.0).await
    }

    #[tool(
        description = "Find audio files on disk not imported into Rekordbox. Compares filesystem contents against the database for each content root."
    )]
    pub(super) async fn scan_orphan_files(
        &self,
        params: Parameters<ScanOrphanFilesParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_scan_orphan_files(self, params.0).await
    }

    #[tool(
        description = "Find tracks not assigned to any playlist. Supports all search filters for scoping."
    )]
    pub(super) async fn scan_playlist_coverage(
        &self,
        params: Parameters<ScanPlaylistCoverageParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_scan_playlist_coverage(self.rekordbox_conn()?, params.0)
    }

    #[tool(
        description = "Detect duplicate tracks by metadata (artist+title) or exact file hash (SHA-256). Groups are stably pageable with offset and page.next_offset; each includes a suggested_keep recommendation based on audio quality.",
        output_schema = rmcp::handler::server::tool::schema_for_output::<ScanDuplicatesOutput>()
            .expect("scan_duplicates output schema should be valid")
    )]
    pub(super) async fn scan_duplicates(
        &self,
        params: Parameters<ScanDuplicatesParams>,
    ) -> Result<CallToolResult, McpError> {
        handle_scan_duplicates(self, params.0).await
    }
}

fn resolve_effective_db_path(configured: Option<&str>) -> Result<PathBuf, String> {
    let path = configured.map(PathBuf::from).ok_or_else(|| {
        "Rekordbox database not found. Set REKORDBOX_DB_PATH to a direct master.db file."
            .to_string()
    })?;
    let metadata = std::fs::symlink_metadata(&path).map_err(|error| {
        format!(
            "Rekordbox database path {} is unavailable: {error}",
            path.display()
        )
    })?;
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "Rekordbox database path {} must be a direct regular file; symlinks are not supported",
            path.display()
        ));
    }
    if !metadata.is_file() {
        return Err(format!(
            "Rekordbox database path {} must be a regular file named master.db",
            path.display()
        ));
    }
    if path.file_name() != Some(std::ffi::OsStr::new("master.db")) {
        return Err(format!(
            "Rekordbox database path {} must name master.db",
            path.display()
        ));
    }

    let canonical = path.canonicalize().map_err(|error| {
        format!(
            "Failed to canonicalize Rekordbox database path {}: {error}",
            path.display()
        )
    })?;
    let canonical_metadata = std::fs::symlink_metadata(&canonical).map_err(|error| {
        format!(
            "Canonical Rekordbox database path {} is unavailable: {error}",
            canonical.display()
        )
    })?;
    if !canonical_metadata.is_file()
        || canonical.file_name() != Some(std::ffi::OsStr::new("master.db"))
    {
        return Err(format!(
            "Canonical Rekordbox database path {} must be a regular file named master.db",
            canonical.display()
        ));
    }
    Ok(canonical)
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for ReklawdboxServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "Rekordbox library management server. Read-only DB access, staged XML export.\n\
                 \n\
                 Before using any workflow tools, call help() to load the matching SOP. Each \
                 workflow has prerequisite steps and a specific tool sequence \u{2014} following the \
                 SOP produces significantly better results than ad-hoc tool use.\n\
                 \n\
                 Call read_library to see the collection.\n\
                 Call help() for workflow menu, help(topic) for the full SOP.\n\
                 \n\
                 Auth-pending errors are agent-actionable, not user-blockers: if a lookup tool \
                 returns an `Auth URL`, open it for the user (`open '<url>'` on macOS) so they \
                 can authorize, then call the same tool again \u{2014} the new session is picked up \
                 automatically. Do not silently fall back to other enrichment sources for \
                 label/catalog data on commercial releases; Discogs is materially more \
                 authoritative there.",
        )
    }
}

#[cfg(test)]
mod retryable_once_init_tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, OnceLock, mpsc};
    use std::time::Duration;

    use super::get_or_try_init_once;

    const TEST_TIMEOUT: Duration = Duration::from_secs(5);

    #[test]
    fn retryable_once_init_error_then_success_is_cached() {
        let cell = OnceLock::new();
        let initializers = AtomicUsize::new(0);

        let first = get_or_try_init_once(&cell, || -> Result<usize, &'static str> {
            initializers.fetch_add(1, Ordering::SeqCst);
            Err("transient failure")
        });
        assert_eq!(first, Err("transient failure"));
        assert!(cell.get().is_none());

        let second = get_or_try_init_once(&cell, || -> Result<usize, &'static str> {
            initializers.fetch_add(1, Ordering::SeqCst);
            Ok(42)
        })
        .expect("second initialization should succeed");
        assert_eq!(*second, 42);

        let third = get_or_try_init_once(&cell, || -> Result<usize, &'static str> {
            initializers.fetch_add(1, Ordering::SeqCst);
            Ok(99)
        })
        .expect("retained value should be reused");
        assert_eq!(*third, 42);
        assert_eq!(initializers.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn retryable_once_init_concurrent_success_returns_retained_value() {
        let cell = Arc::new(OnceLock::new());
        let initializers = Arc::new(AtomicUsize::new(0));
        let (started_tx, started_rx) = mpsc::channel();
        let (completed_tx, completed_rx) = mpsc::channel();
        let (release_one_tx, release_one_rx) = mpsc::channel();
        let (release_two_tx, release_two_rx) = mpsc::channel();

        let spawn_caller = |candidate, release_rx: mpsc::Receiver<()>| {
            let cell = Arc::clone(&cell);
            let initializers = Arc::clone(&initializers);
            let started_tx = started_tx.clone();
            let completed_tx = completed_tx.clone();
            std::thread::spawn(move || {
                let observed = get_or_try_init_once(&cell, || -> Result<usize, ()> {
                    initializers.fetch_add(1, Ordering::SeqCst);
                    started_tx
                        .send(candidate)
                        .expect("start receiver should remain available");
                    release_rx
                        .recv_timeout(TEST_TIMEOUT)
                        .expect("initializer release should be bounded");
                    Ok(candidate)
                })
                .expect("concurrent initializer should return retained success");
                completed_tx
                    .send(*observed)
                    .expect("completion receiver should remain available");
            })
        };

        let first = spawn_caller(11, release_one_rx);
        let second = spawn_caller(22, release_two_rx);
        drop(started_tx);
        drop(completed_tx);

        let started_one = started_rx.recv_timeout(TEST_TIMEOUT);
        let started_two = started_rx.recv_timeout(TEST_TIMEOUT);
        let release_one = release_one_tx.send(());
        let release_two = release_two_tx.send(());
        let completed_one = completed_rx.recv_timeout(TEST_TIMEOUT);
        let completed_two = completed_rx.recv_timeout(TEST_TIMEOUT);
        let first_join = first.join();
        let second_join = second.join();

        assert!(started_one.is_ok(), "first initializer did not start");
        assert!(started_two.is_ok(), "second initializer did not start");
        assert!(
            release_one.is_ok(),
            "first initializer exited before release"
        );
        assert!(
            release_two.is_ok(),
            "second initializer exited before release"
        );
        assert!(first_join.is_ok(), "first initializer thread panicked");
        assert!(second_join.is_ok(), "second initializer thread panicked");
        let completed_one = completed_one.expect("first completion should be bounded");
        let completed_two = completed_two.expect("second completion should be bounded");
        let retained = *cell.get().expect("one successful value should be retained");
        assert_eq!(completed_one, retained);
        assert_eq!(completed_two, retained);
        assert_eq!(initializers.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn retryable_once_init_racing_failure_prefers_visible_success() {
        let cell = Arc::new(OnceLock::new());
        let (started_tx, started_rx) = mpsc::channel();
        let (success_release_tx, success_release_rx) = mpsc::channel();
        let (failure_release_tx, failure_release_rx) = mpsc::channel();
        let (success_tx, success_rx) = mpsc::channel();
        let (failure_tx, failure_rx) = mpsc::channel();

        let success_cell = Arc::clone(&cell);
        let success_started_tx = started_tx.clone();
        let success = std::thread::spawn(move || {
            let observed =
                get_or_try_init_once(&success_cell, || -> Result<usize, &'static str> {
                    success_started_tx
                        .send("success")
                        .expect("start receiver should remain available");
                    success_release_rx
                        .recv_timeout(TEST_TIMEOUT)
                        .expect("success release should be bounded");
                    Ok(73)
                })
                .expect("successful initializer should publish a value");
            success_tx
                .send(*observed)
                .expect("success receiver should remain available");
        });

        let failure_cell = Arc::clone(&cell);
        let failure = std::thread::spawn(move || {
            let observed =
                get_or_try_init_once(&failure_cell, || -> Result<usize, &'static str> {
                    started_tx
                        .send("failure")
                        .expect("start receiver should remain available");
                    failure_release_rx
                        .recv_timeout(TEST_TIMEOUT)
                        .expect("failure release should be bounded");
                    Err("racing failure")
                });
            failure_tx
                .send(observed.copied())
                .expect("failure receiver should remain available");
        });

        let started_one = started_rx.recv_timeout(TEST_TIMEOUT);
        let started_two = started_rx.recv_timeout(TEST_TIMEOUT);
        let success_release = success_release_tx.send(());
        let success_result = success_rx.recv_timeout(TEST_TIMEOUT);
        let failure_release = failure_release_tx.send(());
        let failure_result = failure_rx.recv_timeout(TEST_TIMEOUT);
        let success_join = success.join();
        let failure_join = failure.join();

        assert!(
            started_one.is_ok(),
            "first racing initializer did not start"
        );
        assert!(
            started_two.is_ok(),
            "second racing initializer did not start"
        );
        assert!(
            success_release.is_ok(),
            "success thread exited before release"
        );
        assert!(
            failure_release.is_ok(),
            "failure thread exited before release"
        );
        assert!(success_join.is_ok(), "success thread panicked");
        assert!(failure_join.is_ok(), "failure thread panicked");
        assert_eq!(success_result.expect("success should complete"), 73);
        assert_eq!(failure_result.expect("failure should complete"), Ok(73));
        assert_eq!(cell.get(), Some(&73));
    }
}
