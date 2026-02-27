use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex, OnceLock};

use rmcp::handler::server::tool::ToolRouter;
use rmcp::model::{CallToolResult, Content, ServerCapabilities, ServerInfo};
use rmcp::{ErrorData as McpError, ServerHandler, tool, tool_handler, tool_router};
use rusqlite::Connection;

mod analysis;
mod audio_scan;
mod corpus_helpers;
mod enrichment;
mod essentia;
mod params;
mod resolve;
mod scoring;

use analysis::*;
use audio_scan::*;
use corpus_helpers::*;
use enrichment::*;
use essentia::*;
pub(crate) use essentia::probe_essentia_python_path;
use params::*;
use resolve::*;
use scoring::*;

use crate::audit;
use crate::audio;
use crate::beatport;
use crate::changes::ChangeManager;
use crate::color;
use crate::db;
use crate::discogs;
use crate::genre;
use crate::store;
use crate::tags;
use crate::types::TrackChange;
use crate::xml;

fn internal(msg: String) -> McpError {
    McpError::internal_error(msg, None)
}

/// Inner shared state (not Clone).
struct ServerState {
    db: OnceLock<Result<Mutex<Connection>, String>>,
    internal_db: OnceLock<Result<Mutex<Connection>, String>>,
    essentia_python: OnceLock<Option<String>>,
    essentia_python_override: Mutex<Option<String>>,
    essentia_setup_lock: tokio::sync::Mutex<()>,
    discogs_pending: Mutex<Option<discogs::PendingDeviceSession>>,
    db_path: Option<String>,
    changes: ChangeManager,
    http: reqwest::Client,
}

#[derive(Clone)]
pub struct ReklawdboxServer {
    state: Arc<ServerState>,
    tool_router: ToolRouter<Self>,
}

impl ReklawdboxServer {
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

    fn internal_store_path(&self) -> String {
        std::env::var("CRATE_DIG_STORE_PATH")
            .unwrap_or_else(|_| store::default_path().to_string_lossy().to_string())
    }

    fn essentia_python_path(&self) -> Option<String> {
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

    async fn lookup_discogs_live(
        &self,
        artist: &str,
        title: &str,
        album: Option<&str>,
    ) -> Result<Option<discogs::DiscogsResult>, discogs::LookupError> {
        #[cfg(test)]
        if let Some(result) = take_test_discogs_lookup_override(artist, title, album) {
            return result;
        }

        if let discogs::BrokerConfigResult::InvalidUrl(raw) = discogs::BrokerConfig::from_env() {
            return Err(discogs::LookupError::message(format!(
                "Invalid broker URL in {}: {raw}",
                discogs::BROKER_URL_ENV
            )));
        }

        if let discogs::BrokerConfigResult::Ok(cfg) = discogs::BrokerConfig::from_env() {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;

            let persisted_session = {
                let store = self.internal_conn().map_err(|e| {
                    discogs::LookupError::message(format!("Internal store error: {e}"))
                })?;
                store::get_broker_discogs_session(&store, &cfg.base_url).map_err(|e| {
                    discogs::LookupError::message(format!("Broker session cache read error: {e}"))
                })?
            };

            if let Some(session) = persisted_session {
                if session.expires_at > now {
                    match discogs::lookup_via_broker(
                        &self.state.http,
                        &cfg,
                        &session.session_token,
                        artist,
                        title,
                        album,
                    )
                    .await
                    {
                        Ok(result) => return Ok(result),
                        Err(discogs::LookupError::AuthRequired(_)) => {
                            let store = self.internal_conn().map_err(|e| {
                                discogs::LookupError::message(format!("Internal store error: {e}"))
                            })?;
                            store::clear_broker_discogs_session(&store, &cfg.base_url).map_err(
                                |e| {
                                    discogs::LookupError::message(format!(
                                        "Broker session cache clear error: {e}"
                                    ))
                                },
                            )?;
                        }
                        Err(e) => return Err(e),
                    }
                } else {
                    let store = self.internal_conn().map_err(|e| {
                        discogs::LookupError::message(format!("Internal store error: {e}"))
                    })?;
                    store::clear_broker_discogs_session(&store, &cfg.base_url).map_err(|e| {
                        discogs::LookupError::message(format!(
                            "Broker session cache clear error: {e}"
                        ))
                    })?;
                }
            }

            let pending = {
                let lock = self.state.discogs_pending.lock().map_err(|_| {
                    discogs::LookupError::message("Discogs auth state lock poisoned")
                })?;
                lock.clone()
            };

            if let Some(pending) = pending {
                if pending.expires_at > now {
                    let status = discogs::device_session_status(&self.state.http, &cfg, &pending)
                        .await
                        .map_err(|e| {
                            discogs::LookupError::message(format!(
                                "Discogs broker status error: {e}"
                            ))
                        })?;

                    if status.status == "authorized" || status.status == "finalized" {
                        let finalized =
                            discogs::device_session_finalize(&self.state.http, &cfg, &pending)
                                .await
                                .map_err(|e| {
                                    discogs::LookupError::message(format!(
                                        "Discogs broker finalize error: {e}"
                                    ))
                                })?;

                        {
                            let store = self.internal_conn().map_err(|e| {
                                discogs::LookupError::message(format!("Internal store error: {e}"))
                            })?;
                            store::set_broker_discogs_session(
                                &store,
                                &cfg.base_url,
                                &finalized.session_token,
                                finalized.expires_at,
                            )
                            .map_err(|e| {
                                discogs::LookupError::message(format!(
                                    "Broker session cache write error: {e}"
                                ))
                            })?;
                        }
                        {
                            let mut lock = self.state.discogs_pending.lock().map_err(|_| {
                                discogs::LookupError::message("Discogs auth state lock poisoned")
                            })?;
                            *lock = None;
                        }

                        return discogs::lookup_via_broker(
                            &self.state.http,
                            &cfg,
                            &finalized.session_token,
                            artist,
                            title,
                            album,
                        )
                        .await;
                    }

                    if status.status == "pending" {
                        return Err(discogs::LookupError::AuthRequired(
                            discogs::pending_auth_remediation(&pending),
                        ));
                    }
                }

                let mut lock = self.state.discogs_pending.lock().map_err(|_| {
                    discogs::LookupError::message("Discogs auth state lock poisoned")
                })?;
                *lock = None;
            }

            let started = discogs::device_session_start(&self.state.http, &cfg)
                .await
                .map_err(|e| {
                    discogs::LookupError::message(format!("Discogs broker start error: {e}"))
                })?;
            {
                let mut lock = self.state.discogs_pending.lock().map_err(|_| {
                    discogs::LookupError::message("Discogs auth state lock poisoned")
                })?;
                *lock = Some(started.clone());
            }
            return Err(discogs::LookupError::AuthRequired(
                discogs::pending_auth_remediation(&started),
            ));
        }

        if discogs::legacy_credentials_configured() {
            return discogs::lookup_with_legacy_credentials(&self.state.http, artist, title, album)
                .await
                .map_err(discogs::LookupError::message);
        }

        Err(discogs::LookupError::AuthRequired(
            discogs::missing_auth_remediation(),
        ))
    }

    async fn lookup_beatport_live(
        &self,
        artist: &str,
        title: &str,
    ) -> Result<Option<beatport::BeatportResult>, String> {
        #[cfg(test)]
        if let Some(result) = take_test_beatport_lookup_override(artist, title) {
            return result;
        }

        beatport::lookup(&self.state.http, artist, title)
            .await
            .map_err(|e| e.to_string())
    }
}


use rmcp::handler::server::wrapper::Parameters;

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
        let conn = self.conn()?;
        let mut search = params.0.filters.into_search_params(
            !params.0.include_samples.unwrap_or(false),
            params.0.limit,
            params.0.offset,
        );
        search.playlist = params.0.playlist;
        let tracks =
            db::search_tracks(&conn, &search).map_err(|e| internal(format!("DB error: {e}")))?;
        let json = serde_json::to_string_pretty(&tracks).map_err(|e| internal(format!("{e}")))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Get full details for a specific track by ID")]
    async fn get_track(
        &self,
        params: Parameters<GetTrackParams>,
    ) -> Result<CallToolResult, McpError> {
        let conn = self.conn()?;
        let track =
            db::get_track(&conn, &params.0.track_id).map_err(|e| internal(format!("DB error: {e}")))?;
        match track {
            Some(t) => {
                let json = serde_json::to_string_pretty(&t).map_err(|e| internal(format!("{e}")))?;
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
        let playlists = db::get_playlists(&conn).map_err(|e| internal(format!("DB error: {e}")))?;
        let json = serde_json::to_string_pretty(&playlists).map_err(|e| internal(format!("{e}")))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "List tracks in a specific playlist")]
    async fn get_playlist_tracks(
        &self,
        params: Parameters<GetPlaylistTracksParams>,
    ) -> Result<CallToolResult, McpError> {
        let conn = self.conn()?;
        let tracks = db::get_playlist_tracks(&conn, &params.0.playlist_id, params.0.limit)
            .map_err(|e| internal(format!("DB error: {e}")))?;
        let json = serde_json::to_string_pretty(&tracks).map_err(|e| internal(format!("{e}")))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Get library summary: track count, genre distribution, stats")]
    async fn read_library(&self) -> Result<CallToolResult, McpError> {
        let conn = self.conn()?;
        let stats = db::get_library_stats(&conn).map_err(|e| internal(format!("DB error: {e}")))?;
        let json = serde_json::to_string_pretty(&stats).map_err(|e| internal(format!("{e}")))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Get the configured genre taxonomy")]
    async fn get_genre_taxonomy(&self) -> Result<CallToolResult, McpError> {
        let genres = genre::GENRES;
        let aliases = genre::alias_map();
        let mut result = serde_json::json!({
            "genres": genres,
            "aliases": aliases,
            "description": "Flat genre taxonomy. Not a closed list — arbitrary genres are accepted. This list provides consistency suggestions. Aliases map non-canonical genre names to their canonical forms."
        });
        attach_corpus_provenance(&mut result, consult_genre_workflow_docs());
        let json = serde_json::to_string_pretty(&result).map_err(|e| internal(format!("{e}")))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        description = "Stage changes to track metadata (genre, comments, rating, color). Changes are held in memory until write_xml is called."
    )]
    async fn update_tracks(
        &self,
        params: Parameters<UpdateTracksParams>,
    ) -> Result<CallToolResult, McpError> {
        for c in &params.0.changes {
            if let Some(r) = c.rating
                && (r == 0 || r > 5)
            {
                return Err(McpError::invalid_params(
                    format!("rating must be 1-5, got {r}"),
                    None,
                ));
            }
            if let Some(ref col) = c.color
                && !color::is_valid_color(col)
            {
                let valid: Vec<&str> = color::COLORS.iter().map(|(n, _)| *n).collect();
                return Err(McpError::invalid_params(
                    format!(
                        "unknown color '{}'. Valid colors: {}",
                        col,
                        valid.join(", ")
                    ),
                    None,
                ));
            }
        }

        let mut warnings: Vec<String> = Vec::new();
        for c in &params.0.changes {
            if let Some(ref g) = c.genre
                && !genre::is_known_genre(g)
            {
                warnings.push(format!("'{}' is not in the genre taxonomy", g));
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
                color: c.color.map(|col| {
                    color::canonical_casing(&col)
                        .map(String::from)
                        .unwrap_or(col)
                }),
            })
            .collect();

        let echo: Vec<serde_json::Value> = changes
            .iter()
            .map(|c| {
                serde_json::json!({
                    "track_id": c.track_id,
                    "genre": c.genre,
                    "comments": c.comments,
                    "rating": c.rating,
                    "color": c.color,
                })
            })
            .collect();

        let (staged, total) = self.state.changes.stage(changes);
        let mut result = serde_json::json!({
            "staged": staged,
            "total_pending": total,
            "changes": echo,
        });
        if !warnings.is_empty() {
            result["warnings"] = serde_json::json!(warnings);
        }
        attach_corpus_provenance(&mut result, consult_genre_workflow_docs());
        let json = serde_json::to_string_pretty(&result).map_err(|e| internal(format!("{e}")))?;
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

        let stats = db::get_library_stats(&conn).map_err(|e| internal(format!("DB error: {e}")))?;

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
                let tracks = db::get_tracks_by_exact_genre(&conn, &gc.name, true)
                    .map_err(|e| internal(format!("DB error: {e}")))?;
                for t in tracks {
                    alias_suggestions.push(crate::types::NormalizationSuggestion {
                        track_id: t.id,
                        title: t.title,
                        artist: t.artist,
                        current_genre: gc.name.clone(),
                        suggested_genre: Some(canonical.to_string()),
                        confidence: crate::types::Confidence::Alias,
                    });
                }
            } else if genre::is_known_genre(&gc.name) {
                canonical_items.push(serde_json::json!({
                    "genre": gc.name,
                    "count": gc.count,
                }));
            } else {
                let tracks = db::get_tracks_by_exact_genre(&conn, &gc.name, true)
                    .map_err(|e| internal(format!("DB error: {e}")))?;
                for t in tracks {
                    unknown_items.push(crate::types::NormalizationSuggestion {
                        track_id: t.id,
                        title: t.title,
                        artist: t.artist,
                        current_genre: gc.name.clone(),
                        suggested_genre: None,
                        confidence: crate::types::Confidence::Unknown,
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
        let json = serde_json::to_string_pretty(&result).map_err(|e| internal(format!("{e}")))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Preview all staged changes, showing what will differ from current state")]
    async fn preview_changes(
        &self,
        params: Parameters<PreviewChangesParams>,
    ) -> Result<CallToolResult, McpError> {
        let mut ids = self.state.changes.pending_ids();
        if ids.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(
                "No changes staged.",
            )]));
        }

        // Filter to requested track IDs if provided
        if let Some(ref filter_ids) = params.0.track_ids {
            let filter_set: HashSet<&str> = filter_ids.iter().map(|s| s.as_str()).collect();
            ids.retain(|id| filter_set.contains(id.as_str()));
            if ids.is_empty() {
                return Ok(CallToolResult::success(vec![Content::text(
                    "No staged changes for the specified track IDs.",
                )]));
            }
        }

        let conn = self.conn()?;
        let current_tracks =
            db::get_tracks_by_ids(&conn, &ids).map_err(|e| internal(format!("DB error: {e}")))?;

        let diffs = self.state.changes.preview(&current_tracks);
        if diffs.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(
                "Changes staged but no fields actually differ from current values.",
            )]));
        }

        let json = serde_json::to_string_pretty(&diffs).map_err(|e| internal(format!("{e}")))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        description = "Write staged changes and optional playlists to a Rekordbox-compatible XML file. Runs backup first."
    )]
    async fn write_xml(
        &self,
        params: Parameters<WriteXmlParams>,
    ) -> Result<CallToolResult, McpError> {
        let playlists = params.0.playlists.unwrap_or_default();
        let has_playlists = !playlists.is_empty();
        let snapshot = self.state.changes.take(None);
        if snapshot.is_empty() && !has_playlists {
            let mut result = serde_json::json!({
                "message": "No changes to write.",
                "track_count": 0,
                "changes_applied": 0,
            });
            attach_corpus_provenance(&mut result, consult_xml_workflow_docs());
            let json = serde_json::to_string_pretty(&result).map_err(|e| internal(format!("{e}")))?;
            return Ok(CallToolResult::success(vec![Content::text(json)]));
        }

        let backup_script_env = std::env::var("REKLAWDBOX_BACKUP_SCRIPT")
            .ok()
            .filter(|v| !v.trim().is_empty());
        let backup_script_candidates: Vec<String> = backup_script_env
            .into_iter()
            .chain(["scripts/backup.sh".to_string(), "backup.sh".to_string()])
            .collect();
        let backup_script = backup_script_candidates
            .iter()
            .find(|path| std::path::Path::new(path).exists());
        if let Some(script_path) = backup_script {
            eprintln!("[reklawdbox] Running pre-op backup...");
            let script_path = script_path.to_string();
            match tokio::task::spawn_blocking(move || {
                std::process::Command::new("bash")
                    .arg(&script_path)
                    .arg("--pre-op")
                    .output()
            })
            .await
            {
                Ok(Ok(o)) if o.status.success() => eprintln!("[reklawdbox] Backup completed."),
                Ok(Ok(o)) => {
                    let stderr_out = String::from_utf8_lossy(&o.stderr);
                    eprintln!("[reklawdbox] Backup warning: {stderr_out}");
                }
                Ok(Err(e)) => eprintln!("[reklawdbox] Backup skipped: {e}"),
                Err(e) => eprintln!("[reklawdbox] Backup task failed: {e}"),
            }
        }

        let mut ids = Vec::new();
        let mut seen_ids = HashSet::new();
        for change in &snapshot {
            if seen_ids.insert(change.track_id.clone()) {
                ids.push(change.track_id.clone());
            }
        }
        for playlist in &playlists {
            for track_id in &playlist.track_ids {
                if seen_ids.insert(track_id.clone()) {
                    ids.push(track_id.clone());
                }
            }
        }

        let conn = match self.conn() {
            Ok(conn) => conn,
            Err(e) => {
                self.state.changes.restore(snapshot);
                return Err(e);
            }
        };
        let current_tracks = match db::get_tracks_by_ids(&conn, &ids) {
            Ok(tracks) => tracks,
            Err(e) => {
                self.state.changes.restore(snapshot);
                return Err(internal(format!("DB error: {e}")));
            }
        };
        let found_ids: HashSet<&str> = current_tracks.iter().map(|t| t.id.as_str()).collect();
        let missing_ids: Vec<String> = ids
            .iter()
            .filter(|id| !found_ids.contains(id.as_str()))
            .cloned()
            .collect();
        if !missing_ids.is_empty() {
            self.state.changes.restore(snapshot);
            return Err(internal(format!(
                "Track IDs not found in database: {}",
                missing_ids.join(", ")
            )));
        }
        let ordered_tracks = current_tracks;
        let modified_tracks = self
            .state
            .changes
            .apply_snapshot(&ordered_tracks, &snapshot);
        let playlist_defs: Vec<xml::PlaylistDef> = playlists
            .iter()
            .map(|playlist| xml::PlaylistDef {
                name: playlist.name.clone(),
                track_ids: playlist.track_ids.clone(),
            })
            .collect();

        let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let output_path = params.0.output_path.map(PathBuf::from).unwrap_or_else(|| {
            PathBuf::from(format!("rekordbox-exports/reklawdbox-{timestamp}.xml"))
        });

        if let Err(e) =
            xml::write_xml_with_playlists(&modified_tracks, &playlist_defs, &output_path)
        {
            self.state.changes.restore(snapshot);
            return Err(internal(format!("Write error: {e}")));
        }

        let track_count = modified_tracks.len();
        let changes_applied = snapshot.len();

        let mut result = serde_json::json!({
            "path": output_path.to_string_lossy(),
            "track_count": track_count,
            "changes_applied": changes_applied,
        });
        if has_playlists {
            result["playlist_count"] = serde_json::json!(playlists.len());
        }
        attach_corpus_provenance(&mut result, consult_xml_workflow_docs());
        let json = serde_json::to_string_pretty(&result).map_err(|e| internal(format!("{e}")))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Clear staged changes for specific tracks or all")]
    async fn clear_changes(
        &self,
        params: Parameters<ClearChangesParams>,
    ) -> Result<CallToolResult, McpError> {
        if let Some(ref fields) = params.0.fields {
            for f in fields {
                if crate::types::EditableField::from_str(f.as_str()).is_none() {
                    return Err(McpError::invalid_params(
                        format!(
                            "unknown field '{}'. Valid fields: {}",
                            f,
                            crate::types::EditableField::all_names_csv()
                        ),
                        None,
                    ));
                }
            }
            let (affected, remaining) = self.state.changes.clear_fields(params.0.track_ids, fields);
            let result = serde_json::json!({
                "affected": affected,
                "remaining": remaining,
                "fields_cleared": fields,
            });
            let json = serde_json::to_string_pretty(&result).map_err(|e| internal(format!("{e}")))?;
            Ok(CallToolResult::success(vec![Content::text(json)]))
        } else {
            let (cleared, remaining) = self.state.changes.clear(params.0.track_ids);
            let result = serde_json::json!({
                "cleared": cleared,
                "remaining": remaining,
            });
            let json = serde_json::to_string_pretty(&result).map_err(|e| internal(format!("{e}")))?;
            Ok(CallToolResult::success(vec![Content::text(json)]))
        }
    }

    #[tool(
        description = "Look up a track on Discogs for genre/style enrichment. Returns an object payload with lookup data plus cache metadata (`cache_hit`, optional `cached_at`). On no match, `result` is null. Results are cached. Pass track_id to auto-fill artist/title/album from the library."
    )]
    async fn lookup_discogs(
        &self,
        params: Parameters<LookupDiscogsParams>,
    ) -> Result<CallToolResult, McpError> {
        let force_refresh = params.0.force_refresh.unwrap_or(false);

        // Resolve artist/title/album: from track_id or explicit params
        let (artist, title, album) = if let Some(ref track_id) = params.0.track_id {
            let conn = self.conn()?;
            let track = db::get_track(&conn, track_id)
                .map_err(|e| internal(format!("DB error: {e}")))?
                .ok_or_else(|| {
                    McpError::invalid_params(format!("Track '{track_id}' not found"), None)
                })?;
            let album = params
                .0
                .album
                .or_else(|| (!track.album.is_empty()).then(|| track.album.clone()));
            (
                params.0.artist.unwrap_or(track.artist),
                params.0.title.unwrap_or(track.title),
                album,
            )
        } else {
            let artist = params.0.artist.ok_or_else(|| {
                McpError::invalid_params("artist is required when track_id is not provided", None)
            })?;
            let title = params.0.title.ok_or_else(|| {
                McpError::invalid_params("title is required when track_id is not provided", None)
            })?;
            (artist, title, params.0.album)
        };

        let norm_artist = crate::normalize::normalize(&artist);
        let norm_title = crate::normalize::normalize(&title);

        if !force_refresh {
            let store = self.internal_conn()?;
            if let Some(cached) =
                store::get_enrichment(&store, "discogs", &norm_artist, &norm_title)
                    .map_err(|e| internal(format!("Cache read error: {e}")))?
            {
                let result = match &cached.response_json {
                    Some(json_str) => serde_json::from_str::<serde_json::Value>(json_str)
                        .unwrap_or(serde_json::Value::Null),
                    None => serde_json::Value::Null,
                };
                let result = lookup_output_with_cache_metadata(
                    result,
                    true,
                    Some(cached.created_at.as_str()),
                );
                let json =
                    serde_json::to_string_pretty(&result).map_err(|e| internal(format!("{e}")))?;
                return Ok(CallToolResult::success(vec![Content::text(json)]));
            }
        }

        let result = self
            .lookup_discogs_live(&artist, &title, album.as_deref())
            .await
            .map_err(|e| match e.auth_remediation() {
                Some(remediation) => internal(auth_remediation_message(remediation)),
                None => internal(format!("Discogs error: {e}")),
            })?;

        let (match_quality, response_json) = match &result {
            Some(r) => {
                let quality = if r.fuzzy_match { "fuzzy" } else { "exact" };
                let json = serde_json::to_string(r).map_err(|e| internal(format!("{e}")))?;
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
            .map_err(|e| internal(format!("Cache write error: {e}")))?;
        }

        let output = lookup_output_with_cache_metadata(
            serde_json::to_value(&result).map_err(|e| internal(format!("{e}")))?,
            false,
            None,
        );
        let json = serde_json::to_string_pretty(&output).map_err(|e| internal(format!("{e}")))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        description = "Look up a track on Beatport for genre/BPM/key enrichment. Returns an object payload with lookup data plus cache metadata (`cache_hit`, optional `cached_at`). On no match, `result` is null. Results are cached. Pass track_id to auto-fill artist/title from the library."
    )]
    async fn lookup_beatport(
        &self,
        params: Parameters<LookupBeatportParams>,
    ) -> Result<CallToolResult, McpError> {
        let force_refresh = params.0.force_refresh.unwrap_or(false);

        // Resolve artist/title: from track_id or explicit params
        let (artist, title) = if let Some(ref track_id) = params.0.track_id {
            let conn = self.conn()?;
            let track = db::get_track(&conn, track_id)
                .map_err(|e| internal(format!("DB error: {e}")))?
                .ok_or_else(|| {
                    McpError::invalid_params(format!("Track '{track_id}' not found"), None)
                })?;
            (
                params.0.artist.unwrap_or(track.artist),
                params.0.title.unwrap_or(track.title),
            )
        } else {
            let artist = params.0.artist.ok_or_else(|| {
                McpError::invalid_params("artist is required when track_id is not provided", None)
            })?;
            let title = params.0.title.ok_or_else(|| {
                McpError::invalid_params("title is required when track_id is not provided", None)
            })?;
            (artist, title)
        };

        let norm_artist = crate::normalize::normalize(&artist);
        let norm_title = crate::normalize::normalize(&title);

        if !force_refresh {
            let store = self.internal_conn()?;
            if let Some(cached) =
                store::get_enrichment(&store, "beatport", &norm_artist, &norm_title)
                    .map_err(|e| internal(format!("Cache read error: {e}")))?
            {
                let result = match &cached.response_json {
                    Some(json_str) => serde_json::from_str::<serde_json::Value>(json_str)
                        .unwrap_or(serde_json::Value::Null),
                    None => serde_json::Value::Null,
                };
                let result = lookup_output_with_cache_metadata(
                    result,
                    true,
                    Some(cached.created_at.as_str()),
                );
                let json =
                    serde_json::to_string_pretty(&result).map_err(|e| internal(format!("{e}")))?;
                return Ok(CallToolResult::success(vec![Content::text(json)]));
            }
        }

        let result = self
            .lookup_beatport_live(&artist, &title)
            .await
            .map_err(|e| internal(format!("Beatport error: {e}")))?;

        let (match_quality, response_json) = match &result {
            Some(r) => {
                let json = serde_json::to_string(r).map_err(|e| internal(format!("{e}")))?;
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
            .map_err(|e| internal(format!("Cache write error: {e}")))?;
        }

        let output = lookup_output_with_cache_metadata(
            serde_json::to_value(&result).map_err(|e| internal(format!("{e}")))?,
            false,
            None,
        );
        let json = serde_json::to_string_pretty(&output).map_err(|e| internal(format!("{e}")))?;
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
        let skip_cached = p.skip_cached.unwrap_or(true);
        let force_refresh = p.force_refresh.unwrap_or(false);
        let providers = p
            .providers
            .unwrap_or_else(|| vec![crate::types::Provider::Discogs]);

        let tracks = {
            let conn = self.conn()?;
            resolve_tracks(
                &conn,
                p.track_ids.as_deref(),
                p.playlist_id.as_deref(),
                p.filters,
                p.max_tracks,
                &ResolveTracksOpts { default_max: Some(50), cap: Some(200), exclude_samplers: false },
            )?
        };

        let total_tracks = tracks.len();
        let total = total_tracks.saturating_mul(providers.len());

        let mut enriched = 0usize;
        let mut cached = 0usize;
        let mut skipped = 0usize;
        let mut failed: Vec<serde_json::Value> = Vec::new();
        let mut discogs_auth_error: Option<String> = None;

        for track in &tracks {
            let norm_artist = crate::normalize::normalize(&track.artist);
            let norm_title = crate::normalize::normalize(&track.title);

            for provider in &providers {
                if skip_cached && !force_refresh {
                    let store = self.internal_conn()?;
                    if store::get_enrichment(
                        &store,
                        provider.as_str(),
                        &norm_artist,
                        &norm_title,
                    )
                    .map_err(|e| internal(format!("Cache read error: {e}")))?
                    .is_some()
                    {
                        cached += 1;
                        continue;
                    }
                }

                match provider {
                    crate::types::Provider::Discogs => {
                        if let Some(auth_err) = discogs_auth_error.clone() {
                            failed.push(serde_json::json!({
                                "track_id": track.id,
                                "artist": track.artist,
                                "title": track.title,
                                "provider": provider,
                                "error": auth_err,
                            }));
                            continue;
                        }

                        let album = if track.album.is_empty() {
                            None
                        } else {
                            Some(track.album.as_str())
                        };
                        match self
                            .lookup_discogs_live(&track.artist, &track.title, album)
                            .await
                        {
                            Ok(Some(r)) => {
                                let json_str =
                                    serde_json::to_string(&r).map_err(|e| internal(format!("{e}")))?;
                                let quality = if r.fuzzy_match { "fuzzy" } else { "exact" };
                                let store = self.internal_conn()?;
                                store::set_enrichment(
                                    &store,
                                    provider.as_str(),
                                    &norm_artist,
                                    &norm_title,
                                    Some(quality),
                                    Some(&json_str),
                                )
                                .map_err(|e| internal(format!("Cache write error: {e}")))?;
                                enriched += 1;
                            }
                            Ok(None) => {
                                let store = self.internal_conn()?;
                                store::set_enrichment(
                                    &store,
                                    provider.as_str(),
                                    &norm_artist,
                                    &norm_title,
                                    Some("none"),
                                    None,
                                )
                                .map_err(|e| internal(format!("Cache write error: {e}")))?;
                                skipped += 1;
                            }
                            Err(e) => {
                                let error_message = if let Some(remediation) = e.auth_remediation()
                                {
                                    let msg = auth_remediation_message(remediation);
                                    discogs_auth_error = Some(msg.clone());
                                    msg
                                } else {
                                    e.to_string()
                                };
                                failed.push(serde_json::json!({
                                    "track_id": track.id,
                                    "artist": track.artist,
                                    "title": track.title,
                                    "provider": provider.as_str(),
                                    "error": error_message,
                                }));
                            }
                        }
                    }
                    crate::types::Provider::Beatport => {
                        match beatport::lookup(&self.state.http, &track.artist, &track.title).await
                        {
                            Ok(Some(r)) => {
                                let json_str =
                                    serde_json::to_string(&r).map_err(|e| internal(format!("{e}")))?;
                                let store = self.internal_conn()?;
                                store::set_enrichment(
                                    &store,
                                    provider.as_str(),
                                    &norm_artist,
                                    &norm_title,
                                    Some("exact"),
                                    Some(&json_str),
                                )
                                .map_err(|e| internal(format!("Cache write error: {e}")))?;
                                enriched += 1;
                            }
                            Ok(None) => {
                                let store = self.internal_conn()?;
                                store::set_enrichment(
                                    &store,
                                    provider.as_str(),
                                    &norm_artist,
                                    &norm_title,
                                    Some("none"),
                                    None,
                                )
                                .map_err(|e| internal(format!("Cache write error: {e}")))?;
                                skipped += 1;
                            }
                            Err(e) => {
                                failed.push(serde_json::json!({
                                    "track_id": track.id,
                                    "artist": track.artist,
                                    "title": track.title,
                                    "provider": provider.as_str(),
                                    "error": e.to_string(),
                                }));
                            }
                        }
                    }
                }
            }
        }

        let result = serde_json::json!({
            "summary": {
                "tracks_total": total_tracks,
                "total": total,
                "enriched": enriched,
                "cached": cached,
                "skipped": skipped,
                "failed": failed.len(),
            },
            "failures": failed,
        });
        let json = serde_json::to_string_pretty(&result).map_err(|e| internal(format!("{e}")))?;
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
                .map_err(|e| internal(format!("DB error: {e}")))?
                .ok_or_else(|| {
                    McpError::invalid_params(
                        format!("Track '{}' not found", params.0.track_id),
                        None,
                    )
                })?
        };

        let file_path = resolve_file_path(&track.file_path)?;
        let metadata = std::fs::metadata(&file_path)
            .map_err(|e| internal(format!("Cannot stat file '{}': {e}", file_path)))?;
        let file_size = metadata.len() as i64;
        let file_mtime = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        // Stratum-dsp: check cache then analyze
        let stratum_cached = if skip_cached {
            let store = self.internal_conn()?;
            check_analysis_cache(&store, &file_path, audio::ANALYZER_STRATUM, file_size, file_mtime)
                .map_err(internal)?
        } else {
            None
        };

        let (stratum_dsp, stratum_cache_hit) = if let Some(json_str) = stratum_cached {
            let val = serde_json::from_str(&json_str).map_err(|e| internal(format!("Cache parse error: {e}")))?;
            (val, true)
        } else {
            let analysis = analyze_stratum(&file_path).await.map_err(internal)?;
            let features_json = serde_json::to_string(&analysis).map_err(|e| internal(format!("{e}")))?;
            let store = self.internal_conn()?;
            store::set_audio_analysis(&store, &file_path, audio::ANALYZER_STRATUM, file_size, file_mtime, &analysis.analyzer_version, &features_json)
                .map_err(|e| internal(format!("Cache write error: {e}")))?;
            (serde_json::to_value(&analysis).map_err(|e| internal(format!("{e}")))?, false)
        };

        // Essentia: check cache then analyze
        let essentia_python = self.essentia_python_path();
        let essentia_available = essentia_python.is_some();
        let mut essentia: Option<serde_json::Value> = None;
        let mut essentia_cache_hit: Option<bool> = None;
        let mut essentia_error: Option<String> = None;

        if let Some(python_path) = essentia_python.as_deref() {
            let essentia_cached = if skip_cached {
                let store = self.internal_conn()?;
                check_analysis_cache(&store, &file_path, audio::ANALYZER_ESSENTIA, file_size, file_mtime)
                    .map_err(internal)?
            } else {
                None
            };

            if let Some(json_str) = essentia_cached {
                essentia = Some(serde_json::from_str(&json_str).map_err(|e| internal(format!("Cache parse error: {e}")))?);
                essentia_cache_hit = Some(true);
            } else {
                match audio::run_essentia(python_path, &file_path).await.map_err(|e| e.to_string()) {
                    Ok(features) => {
                        let version = if features.analyzer_version.is_empty() { "unknown" } else { &features.analyzer_version };
                        let features_json = serde_json::to_string(&features).map_err(|e| internal(format!("{e}")))?;
                        let store = self.internal_conn()?;
                        store::set_audio_analysis(&store, &file_path, audio::ANALYZER_ESSENTIA, file_size, file_mtime, version, &features_json)
                            .map_err(|e| internal(format!("Cache write error: {e}")))?;
                        essentia = Some(serde_json::to_value(&features).map_err(|e| internal(format!("{e}")))?);
                        essentia_cache_hit = Some(false);
                    }
                    Err(e) => essentia_error = Some(e),
                }
            }
        }

        let mut result = serde_json::json!({
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
        if !essentia_available {
            result["essentia_setup_hint"] = serde_json::Value::String(essentia_setup_hint());
        }
        let json = serde_json::to_string_pretty(&result).map_err(|e| internal(format!("{e}")))?;
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
        let skip_cached = p.skip_cached.unwrap_or(true);

        let tracks = {
            let conn = self.conn()?;
            resolve_tracks(
                &conn,
                p.track_ids.as_deref(),
                p.playlist_id.as_deref(),
                p.filters,
                p.max_tracks,
                &ResolveTracksOpts { default_max: Some(20), cap: Some(200), exclude_samplers: false },
            )?
        };

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
                        "analyzer": audio::ANALYZER_STRATUM,
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
                        "analyzer": audio::ANALYZER_STRATUM,
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

            // Stratum-dsp: check cache then analyze
            let mut stratum_dsp: Option<serde_json::Value> = None;
            let mut stratum_cache_hit = false;

            if skip_cached {
                let store = self.internal_conn()?;
                match check_analysis_cache(&store, &file_path, audio::ANALYZER_STRATUM, file_size, file_mtime) {
                    Ok(Some(json_str)) => match serde_json::from_str(&json_str) {
                        Ok(val) => {
                            stratum_dsp = Some(val);
                            stratum_cache_hit = true;
                            cached += 1;
                        }
                        Err(e) => {
                            failed.push(serde_json::json!({
                                "track_id": track.id, "artist": track.artist,
                                "title": track.title, "analyzer": audio::ANALYZER_STRATUM,
                                "error": format!("Cache parse error: {e}"),
                            }));
                            continue;
                        }
                    },
                    Ok(None) => {}
                    Err(e) => {
                        failed.push(serde_json::json!({
                            "track_id": track.id, "artist": track.artist,
                            "title": track.title, "analyzer": audio::ANALYZER_STRATUM,
                            "error": e,
                        }));
                        continue;
                    }
                }
            }

            if stratum_dsp.is_none() {
                match analyze_stratum(&file_path).await {
                    Ok(analysis) => {
                        let features_json = serde_json::to_string(&analysis).map_err(|e| internal(format!("{e}")))?;
                        let store = self.internal_conn()?;
                        store::set_audio_analysis(&store, &file_path, audio::ANALYZER_STRATUM, file_size, file_mtime, &analysis.analyzer_version, &features_json)
                            .map_err(|e| internal(format!("Cache write error: {e}")))?;
                        stratum_dsp = Some(serde_json::to_value(&analysis).map_err(|e| internal(format!("{e}")))?);
                        analyzed += 1;
                    }
                    Err(e) => {
                        failed.push(serde_json::json!({
                            "track_id": track.id, "artist": track.artist,
                            "title": track.title, "analyzer": audio::ANALYZER_STRATUM,
                            "error": e,
                        }));
                        continue;
                    }
                }
            }

            rows.push(BatchTrackAnalysis {
                track_id: track.id.clone(),
                title: track.title.clone(),
                artist: track.artist.clone(),
                file_path,
                file_size,
                file_mtime,
                stratum_dsp: stratum_dsp
                    .ok_or_else(|| internal("Missing stratum-dsp result in batch".to_string()))?,
                stratum_cache_hit,
                essentia: None,
                essentia_cache_hit: None,
                essentia_error: None,
            });
        }

        // Essentia pass
        let essentia_python = self.essentia_python_path();
        let essentia_available = essentia_python.is_some();

        if let Some(python_path) = essentia_python.as_deref() {
            for row in &mut rows {
                if skip_cached {
                    let store = self.internal_conn()?;
                    match check_analysis_cache(&store, &row.file_path, audio::ANALYZER_ESSENTIA, row.file_size, row.file_mtime) {
                        Ok(Some(json_str)) => match serde_json::from_str(&json_str) {
                            Ok(val) => {
                                row.essentia = Some(val);
                                row.essentia_cache_hit = Some(true);
                                essentia_cached += 1;
                                continue;
                            }
                            Err(e) => {
                                let msg = format!("Cache parse error: {e}");
                                row.essentia_error = Some(msg.clone());
                                essentia_failed += 1;
                                failed.push(serde_json::json!({
                                    "track_id": &row.track_id, "artist": &row.artist,
                                    "title": &row.title, "analyzer": audio::ANALYZER_ESSENTIA, "error": msg,
                                }));
                                continue;
                            }
                        },
                        Ok(None) => {}
                        Err(e) => {
                            row.essentia_error = Some(e.clone());
                            essentia_failed += 1;
                            failed.push(serde_json::json!({
                                "track_id": &row.track_id, "artist": &row.artist,
                                "title": &row.title, "analyzer": audio::ANALYZER_ESSENTIA, "error": e,
                            }));
                            continue;
                        }
                    }
                }

                match audio::run_essentia(python_path, &row.file_path).await.map_err(|e| e.to_string()) {
                    Ok(features) => {
                        let version = if features.analyzer_version.is_empty() { "unknown" } else { &features.analyzer_version };
                        let features_json = serde_json::to_string(&features).map_err(|e| internal(format!("{e}")))?;
                        let store = self.internal_conn()?;
                        store::set_audio_analysis(&store, &row.file_path, audio::ANALYZER_ESSENTIA, row.file_size, row.file_mtime, version, &features_json)
                            .map_err(|e| internal(format!("Cache write error: {e}")))?;
                        row.essentia = Some(serde_json::to_value(&features).map_err(|e| internal(format!("{e}")))?);
                        row.essentia_cache_hit = Some(false);
                        essentia_analyzed += 1;
                    }
                    Err(e) => {
                        row.essentia_error = Some(e.clone());
                        essentia_failed += 1;
                        failed.push(serde_json::json!({
                            "track_id": &row.track_id, "artist": &row.artist,
                            "title": &row.title, "analyzer": audio::ANALYZER_ESSENTIA, "error": e,
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

        let mut result = serde_json::json!({
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
        if !essentia_available {
            result["essentia_setup_hint"] = serde_json::Value::String(essentia_setup_hint());
        }
        let json = serde_json::to_string_pretty(&result).map_err(|e| internal(format!("{e}")))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        description = "Install Essentia into a managed Python venv. Call this when analyze_track_audio reports essentia_available: false. Creates a venv, installs essentia via pip, and makes it available immediately (no restart needed)."
    )]
    async fn setup_essentia(&self) -> Result<CallToolResult, McpError> {
        // Serialize concurrent setup calls — only one install at a time
        let _setup_guard = self.state.essentia_setup_lock.lock().await;

        // Check if already available (validate to catch stale overrides)
        if let Some(path) = self.essentia_python_path() {
            if validate_essentia_python(&path) {
                let result = serde_json::json!({
                    "status": "already_installed",
                    "python_path": path,
                    "message": "Essentia is already available.",
                });
                let json =
                    serde_json::to_string_pretty(&result).map_err(|e| internal(format!("{e}")))?;
                return Ok(CallToolResult::success(vec![Content::text(json)]));
            }
            // Stale override — clear it and proceed with fresh install
            if let Ok(mut guard) = self.state.essentia_python_override.lock() {
                *guard = None;
            }
        }

        let venv_dir = essentia_venv_dir()
            .ok_or_else(|| internal("Cannot determine home directory for venv location".to_string()))?;

        // Find a suitable Python 3 and try venv+pip with each candidate,
        // falling through to the next on failure
        let python_candidates: &[&str] = &[
            "python3.13",
            "python3.12",
            "python3.11",
            "python3.10",
            "python3.9",
            "python3",
        ];

        let mut last_error = String::new();

        for &python_bin in python_candidates {
            // Check this candidate exists
            let bin_ok = tokio::task::spawn_blocking({
                let bin = python_bin.to_string();
                move || {
                    std::process::Command::new(&bin)
                        .args(["--version"])
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .status()
                        .map(|s| s.success())
                        .unwrap_or(false)
                }
            })
            .await
            .unwrap_or(false);

            if !bin_ok {
                continue;
            }

            // Create parent directories
            if let Some(parent) = venv_dir.parent() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    internal(format!(
                        "Failed to create directory {}: {e}",
                        parent.display()
                    ))
                })?;
            }

            // Create venv (--clear ensures a fresh start if a broken venv exists)
            let venv_dir_str = venv_dir.to_string_lossy().to_string();
            let venv_output = tokio::task::spawn_blocking({
                let bin = python_bin.to_string();
                let dir = venv_dir_str.clone();
                move || {
                    std::process::Command::new(&bin)
                        .args(["-m", "venv", "--clear", &dir])
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped())
                        .output()
                }
            })
            .await
            .map_err(|e| internal(format!("venv task failed: {e}")))?
            .map_err(|e| internal(format!("Failed to run {python_bin} -m venv: {e}")))?;

            if !venv_output.status.success() {
                last_error = format!(
                    "{python_bin}: venv creation failed: {}",
                    String::from_utf8_lossy(&venv_output.stderr)
                );
                continue;
            }

            let venv_pip = venv_dir.join("bin/pip");
            let venv_python = venv_dir.join("bin/python");

            // Install essentia
            let pip_output = tokio::task::spawn_blocking({
                let pip = venv_pip.clone();
                move || {
                    std::process::Command::new(&pip)
                        .args(["install", "--pre", "essentia"])
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped())
                        .output()
                }
            })
            .await
            .map_err(|e| internal(format!("pip task failed: {e}")))?
            .map_err(|e| internal(format!("Failed to run pip install: {e}")))?;

            if !pip_output.status.success() {
                last_error = format!(
                    "{python_bin}: pip install essentia failed: {}",
                    String::from_utf8_lossy(&pip_output.stderr)
                );
                continue;
            }

            // Validate the installation
            let venv_python_str = venv_python.to_string_lossy().to_string();
            let validate_output = tokio::task::spawn_blocking({
                let py = venv_python_str.clone();
                move || {
                    std::process::Command::new(&py)
                        .args(["-c", ESSENTIA_IMPORT_CHECK_SCRIPT])
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped())
                        .output()
                }
            })
            .await
            .map_err(|e| internal(format!("validate task failed: {e}")))?
            .map_err(|e| internal(format!("Failed to validate essentia installation: {e}")))?;

            if !validate_output.status.success() {
                last_error = format!(
                    "{python_bin}: Essentia installed but import validation failed: {}",
                    String::from_utf8_lossy(&validate_output.stderr)
                );
                continue;
            }

            let version = String::from_utf8_lossy(&validate_output.stdout)
                .trim()
                .to_string();

            // Set the override so it's available immediately (no restart)
            let mut guard = self
                .state
                .essentia_python_override
                .lock()
                .map_err(|_| internal("essentia override lock poisoned".to_string()))?;
            *guard = Some(venv_python_str.clone());
            drop(guard);

            let result = serde_json::json!({
                "status": "installed",
                "python_path": venv_python_str,
                "python_bin_used": python_bin,
                "essentia_version": version,
                "venv_dir": venv_dir.to_string_lossy(),
                "message": "Essentia installed successfully. Audio analysis will now include Essentia features — no restart needed.",
            });
            let json = serde_json::to_string_pretty(&result).map_err(|e| internal(format!("{e}")))?;
            return Ok(CallToolResult::success(vec![Content::text(json)]));
        }

        Err(internal(format!(
            "All Python candidates failed. Last error: {last_error}"
        )))
    }

    #[tool(
        description = "Score a single transition between two tracks using key, BPM, energy, genre, brightness, and rhythm compatibility."
    )]
    async fn score_transition(
        &self,
        params: Parameters<ScoreTransitionParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let priority = p.priority.unwrap_or(SetPriority::Balanced);

        let (from_track, to_track) = {
            let conn = self.conn()?;
            let from = db::get_track(&conn, &p.from_track_id)
                .map_err(|e| internal(format!("DB error: {e}")))?
                .ok_or_else(|| {
                    McpError::invalid_params(format!("Track '{}' not found", p.from_track_id), None)
                })?;
            let to = db::get_track(&conn, &p.to_track_id)
                .map_err(|e| internal(format!("DB error: {e}")))?
                .ok_or_else(|| {
                    McpError::invalid_params(format!("Track '{}' not found", p.to_track_id), None)
                })?;
            (from, to)
        };

        let (from_profile, to_profile) = {
            let store = self.internal_conn()?;
            let from = build_track_profile(from_track, &store)
                .map_err(|e| internal(format!("Failed to build source track profile: {e}")))?;
            let to = build_track_profile(to_track, &store)
                .map_err(|e| internal(format!("Failed to build destination track profile: {e}")))?;
            (from, to)
        };

        let master_tempo = p.master_tempo.unwrap_or(true);
        let scores = score_transition_profiles(
            &from_profile,
            &to_profile,
            p.energy_phase,
            p.energy_phase,
            priority,
            master_tempo,
            p.harmonic_style,
            &ScoringContext::default(),
        );

        let result = serde_json::json!({
            "from": {
                "track_id": from_profile.track.id,
                "title": from_profile.track.title,
                "artist": from_profile.track.artist,
                "key": from_profile.key_display,
                "bpm": round_score(from_profile.bpm),
                "energy": round_score(from_profile.energy),
                "genre": from_profile.track.genre,
            },
            "to": {
                "track_id": to_profile.track.id,
                "title": to_profile.track.title,
                "artist": to_profile.track.artist,
                "key": to_profile.key_display,
                "bpm": round_score(to_profile.bpm),
                "energy": round_score(to_profile.energy),
                "genre": to_profile.track.genre,
            },
            "scores": scores.to_json(),
        });

        let json = serde_json::to_string_pretty(&result).map_err(|e| internal(format!("{e}")))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        description = "Generate 2-3 candidate set orderings from a track pool using greedy heuristic sequencing."
    )]
    async fn build_set(
        &self,
        params: Parameters<BuildSetParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        if p.track_ids.is_empty() {
            return Err(McpError::invalid_params(
                "track_ids must include at least one track".to_string(),
                None,
            ));
        }
        if p.target_tracks == 0 {
            return Err(McpError::invalid_params(
                "target_tracks must be at least 1".to_string(),
                None,
            ));
        }

        let mut seen = HashSet::new();
        let deduped_ids: Vec<String> = p
            .track_ids
            .into_iter()
            .filter(|track_id| seen.insert(track_id.clone()))
            .collect();
        if deduped_ids.is_empty() {
            return Err(McpError::invalid_params(
                "track_ids must include at least one unique track ID".to_string(),
                None,
            ));
        }

        let requested_candidates = p.candidates.unwrap_or(2).clamp(1, 3) as usize;
        let requested_target = p.target_tracks as usize;
        let priority = p.priority.unwrap_or(SetPriority::Balanced);

        let tracks = {
            let conn = self.conn()?;
            db::get_tracks_by_ids(&conn, &deduped_ids).map_err(|e| internal(format!("DB error: {e}")))?
        };
        if tracks.is_empty() {
            return Err(McpError::invalid_params(
                "No valid tracks found for provided track_ids".to_string(),
                None,
            ));
        }

        let mut profiles_by_id: HashMap<String, TrackProfile> = HashMap::new();
        {
            let store = self.internal_conn()?;
            for track in tracks {
                let profile = build_track_profile(track, &store)
                    .map_err(|e| internal(format!("Failed to build track profile: {e}")))?;
                profiles_by_id.insert(profile.track.id.clone(), profile);
            }
        }

        if let Some(start_track_id) = p.start_track_id.as_deref()
            && !profiles_by_id.contains_key(start_track_id)
        {
            return Err(McpError::invalid_params(
                format!("start_track_id '{start_track_id}' is not in track_ids"),
                None,
            ));
        }

        let actual_target = requested_target.min(profiles_by_id.len());
        let phases = match p.energy_curve.as_ref() {
            Some(EnergyCurveInput::Custom(_)) => {
                let requested_phases =
                    resolve_energy_curve(p.energy_curve.as_ref(), requested_target).map_err(
                        |e| McpError::invalid_params(format!("Invalid energy_curve: {e}"), None),
                    )?;
                requested_phases.into_iter().take(actual_target).collect()
            }
            _ => resolve_energy_curve(p.energy_curve.as_ref(), actual_target).map_err(|e| {
                McpError::invalid_params(format!("Invalid energy_curve: {e}"), None)
            })?,
        };
        let effective_candidates = if profiles_by_id.len() <= actual_target {
            1
        } else {
            requested_candidates
        };
        let start_tracks = select_start_track_ids(
            &profiles_by_id,
            effective_candidates,
            phases[0],
            p.start_track_id.as_deref(),
        );

        let master_tempo = p.master_tempo.unwrap_or(true);
        let harmonic_style = p.harmonic_style;
        let bpm_drift_limit = p.bpm_drift_limit.unwrap_or(15.0);
        let mut candidates = Vec::with_capacity(effective_candidates);
        for candidate_index in 0..effective_candidates {
            let start_track_id = start_tracks[candidate_index % start_tracks.len()].clone();
            let plan = build_candidate_plan(
                &profiles_by_id,
                &start_track_id,
                actual_target,
                &phases,
                priority,
                candidate_index,
                master_tempo,
                harmonic_style,
                bpm_drift_limit,
            );

            let tracks_json: Vec<serde_json::Value> = plan
                .ordered_ids
                .iter()
                .filter_map(|track_id| profiles_by_id.get(track_id))
                .map(|profile| {
                    serde_json::json!({
                        "track_id": profile.track.id,
                        "title": profile.track.title,
                        "artist": profile.track.artist,
                        "key": profile.key_display,
                        "bpm": profile.bpm,
                        "energy": profile.energy,
                        "genre": profile.track.genre,
                    })
                })
                .collect();

            let transitions_json: Vec<serde_json::Value> = plan
                .transitions
                .iter()
                .map(|transition| {
                    serde_json::json!({
                        "from_index": transition.from_index,
                        "to_index": transition.to_index,
                        "scores": transition.scores.to_json(),
                    })
                })
                .collect();

            let total_seconds: i32 = plan
                .ordered_ids
                .iter()
                .filter_map(|track_id| profiles_by_id.get(track_id))
                .map(|profile| {
                    if profile.track.length > 0 {
                        profile.track.length
                    } else {
                        6 * 60
                    }
                })
                .sum();
            let estimated_duration_minutes = (total_seconds as f64 / 60.0).round() as i64;

            let mean_composite = if plan.transitions.is_empty() {
                0.0
            } else {
                plan.transitions
                    .iter()
                    .map(|transition| transition.scores.composite)
                    .sum::<f64>()
                    / plan.transitions.len() as f64
            };
            let set_score = round_score(mean_composite * 10.0);

            let id = ((b'A' + (candidate_index as u8)) as char).to_string();
            candidates.push(serde_json::json!({
                "id": id,
                "tracks": tracks_json,
                "transitions": transitions_json,
                "set_score": set_score,
                "estimated_duration_minutes": estimated_duration_minutes,
            }));
        }

        let result = serde_json::json!({
            "candidates": candidates,
            "pool_size": profiles_by_id.len(),
            "tracks_used": actual_target,
        });
        let json = serde_json::to_string_pretty(&result).map_err(|e| internal(format!("{e}")))?;
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
                .map_err(|e| internal(format!("DB error: {e}")))?
                .ok_or_else(|| {
                    McpError::invalid_params(
                        format!("Track '{}' not found", params.0.track_id),
                        None,
                    )
                })?
        };

        let norm_artist = crate::normalize::normalize(&track.artist);
        let norm_title = crate::normalize::normalize(&track.title);

        let essentia_installed = self.essentia_python_path().is_some();

        let (discogs_cache, beatport_cache, stratum_cache, essentia_cache) = {
            let store = self.internal_conn()?;
            let dc = store::get_enrichment(&store, "discogs", &norm_artist, &norm_title)
                .map_err(|e| internal(format!("Cache read error: {e}")))?;
            let bc = store::get_enrichment(&store, "beatport", &norm_artist, &norm_title)
                .map_err(|e| internal(format!("Cache read error: {e}")))?;
            let audio_cache_key =
                resolve_file_path(&track.file_path).unwrap_or_else(|_| track.file_path.clone());
            let sc = store::get_audio_analysis(&store, &audio_cache_key, audio::ANALYZER_STRATUM)
                .map_err(|e| internal(format!("Cache read error: {e}")))?;
            let ec = store::get_audio_analysis(&store, &audio_cache_key, audio::ANALYZER_ESSENTIA)
                .map_err(|e| internal(format!("Cache read error: {e}")))?;
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

        let json = serde_json::to_string_pretty(&result).map_err(|e| internal(format!("{e}")))?;
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

        let tracks = {
            let conn = self.conn()?;
            resolve_tracks(
                &conn,
                p.track_ids.as_deref(),
                p.playlist_id.as_deref(),
                p.filters,
                p.max_tracks,
                &ResolveTracksOpts { default_max: Some(50), cap: Some(200), exclude_samplers: false },
            )?
        };

        let essentia_installed = self.essentia_python_path().is_some();
        let mut results = Vec::with_capacity(tracks.len());
        for track in &tracks {
            let norm_artist = crate::normalize::normalize(&track.artist);
            let norm_title = crate::normalize::normalize(&track.title);

            let (discogs_cache, beatport_cache, stratum_cache, essentia_cache) = {
                let store = self.internal_conn()?;
                let dc = store::get_enrichment(&store, "discogs", &norm_artist, &norm_title)
                    .map_err(|e| internal(format!("Cache read error: {e}")))?;
                let bc = store::get_enrichment(&store, "beatport", &norm_artist, &norm_title)
                    .map_err(|e| internal(format!("Cache read error: {e}")))?;
                let audio_cache_key =
                    resolve_file_path(&track.file_path).unwrap_or_else(|_| track.file_path.clone());
                let sc = store::get_audio_analysis(&store, &audio_cache_key, audio::ANALYZER_STRATUM)
                    .map_err(|e| internal(format!("Cache read error: {e}")))?;
                let ec = store::get_audio_analysis(&store, &audio_cache_key, audio::ANALYZER_ESSENTIA)
                    .map_err(|e| internal(format!("Cache read error: {e}")))?;
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

        let json = serde_json::to_string_pretty(&results).map_err(|e| internal(format!("{e}")))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        description = "Report cache completeness for a filtered track scope. Cache-only — no external calls."
    )]
    async fn cache_coverage(
        &self,
        params: Parameters<ResolveTracksDataParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let filter_description = describe_resolve_scope(&p);

        let (total_tracks, tracks) = {
            let conn = self.conn()?;
            let sample_prefix = format!("%{}%", db::escape_like(db::SAMPLER_PATH_FRAGMENT));
            let total_tracks: usize = conn
                .query_row(
                    "SELECT COUNT(*) FROM djmdContent
                     WHERE rb_local_deleted = 0
                       AND FolderPath NOT LIKE ?1 ESCAPE '\\'",
                    rusqlite::params![sample_prefix],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|e| internal(format!("DB error: {e}")))?
                .max(0) as usize;

            let tracks = resolve_tracks(
                &conn,
                p.track_ids.as_deref(),
                p.playlist_id.as_deref(),
                p.filters,
                p.max_tracks,
                &ResolveTracksOpts { default_max: None, cap: None, exclude_samplers: true },
            )?;

            (total_tracks, tracks)
        };

        let matched_tracks = tracks.len();
        let essentia_installed = self.essentia_python_path().is_some();

        let mut stratum_cached = 0usize;
        let mut essentia_cached = 0usize;
        let mut discogs_cached = 0usize;
        let mut beatport_cached = 0usize;
        let mut no_audio_analysis = 0usize;
        let mut no_enrichment = 0usize;
        let mut no_data_at_all = 0usize;

        {
            let store = self.internal_conn()?;
            for track in &tracks {
                let norm_artist = crate::normalize::normalize(&track.artist);
                let norm_title = crate::normalize::normalize(&track.title);
                let audio_cache_key =
                    resolve_file_path(&track.file_path).unwrap_or_else(|_| track.file_path.clone());

                let has_discogs =
                    store::get_enrichment(&store, "discogs", &norm_artist, &norm_title)
                        .map_err(|e| internal(format!("Cache read error: {e}")))?
                        .is_some();
                let has_beatport =
                    store::get_enrichment(&store, "beatport", &norm_artist, &norm_title)
                        .map_err(|e| internal(format!("Cache read error: {e}")))?
                        .is_some();
                let has_stratum =
                    store::get_audio_analysis(&store, &audio_cache_key, audio::ANALYZER_STRATUM)
                        .map_err(|e| internal(format!("Cache read error: {e}")))?
                        .is_some();
                let has_essentia = store::get_audio_analysis(&store, &audio_cache_key, audio::ANALYZER_ESSENTIA)
                    .map_err(|e| internal(format!("Cache read error: {e}")))?
                    .is_some();

                if has_stratum {
                    stratum_cached += 1;
                }
                if has_essentia {
                    essentia_cached += 1;
                }
                if has_discogs {
                    discogs_cached += 1;
                }
                if has_beatport {
                    beatport_cached += 1;
                }
                if !has_stratum {
                    no_audio_analysis += 1;
                }
                if !has_discogs && !has_beatport {
                    no_enrichment += 1;
                }
                if !has_stratum && !has_essentia && !has_discogs && !has_beatport {
                    no_data_at_all += 1;
                }
            }
        }

        let result = serde_json::json!({
            "scope": {
                "total_tracks": total_tracks,
                "filter_description": filter_description,
                "matched_tracks": matched_tracks,
            },
            "coverage": {
                "stratum_dsp": {
                    "cached": stratum_cached,
                    "percent": to_percent(stratum_cached, matched_tracks),
                },
                "essentia": {
                    "cached": essentia_cached,
                    "percent": to_percent(essentia_cached, matched_tracks),
                    "installed": essentia_installed,
                },
                "discogs": {
                    "cached": discogs_cached,
                    "percent": to_percent(discogs_cached, matched_tracks),
                },
                "beatport": {
                    "cached": beatport_cached,
                    "percent": to_percent(beatport_cached, matched_tracks),
                },
            },
            "gaps": {
                "no_audio_analysis": no_audio_analysis,
                "no_enrichment": no_enrichment,
                "no_data_at_all": no_data_at_all,
            },
        });

        let json = serde_json::to_string_pretty(&result).map_err(|e| internal(format!("{e}")))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
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
        let p = params.0;

        // Validate exactly one selector
        let selector_count = [p.paths.is_some(), p.track_ids.is_some(), p.directory.is_some()]
            .iter()
            .filter(|&&v| v)
            .count();
        if selector_count != 1 {
            return Err(McpError::invalid_params(
                "Provide exactly one of: paths, track_ids, directory".to_string(),
                None,
            ));
        }

        let limit = p.limit.unwrap_or(200).min(2000);
        let include_cover_art = p.include_cover_art.unwrap_or(false);
        let fields = p.fields;

        // Resolve file paths from the chosen selector
        let mut inline_errors: Vec<tags::FileReadResult> = Vec::new();
        let mut file_paths: Vec<String> = if let Some(paths) = p.paths {
            paths
        } else if let Some(track_ids) = p.track_ids {
            let conn = self.conn()?;
            let mut resolved = Vec::with_capacity(track_ids.len());
            for id in &track_ids {
                match db::get_track(&conn, id) {
                    Ok(Some(track)) => match resolve_file_path(&track.file_path) {
                        Ok(path) => resolved.push(path),
                        Err(e) => inline_errors.push(tags::FileReadResult::Error {
                            path: format!("track_id:{id}"),
                            error: format!("Failed to resolve path: {e}"),
                        }),
                    },
                    Ok(None) => {
                        inline_errors.push(tags::FileReadResult::Error {
                            path: format!("track_id:{id}"),
                            error: format!("Track '{id}' not found"),
                        });
                    }
                    Err(e) => {
                        inline_errors.push(tags::FileReadResult::Error {
                            path: format!("track_id:{id}"),
                            error: format!("DB error: {e}"),
                        });
                    }
                }
            }
            resolved
        } else if let Some(directory) = p.directory {
            let recursive = p.recursive.unwrap_or(false);
            let glob_pattern = p.glob.clone();
            scan_audio_directory(&directory, recursive, glob_pattern.as_deref())
                .map_err(internal)?
        } else {
            unreachable!()
        };

        file_paths.truncate(limit);

        // Read tags concurrently with a semaphore (max 8 concurrent)
        let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(8));
        let mut handles = Vec::with_capacity(file_paths.len());

        for file_path in file_paths {
            let sem = semaphore.clone();
            let fields_clone = fields.clone();
            handles.push(tokio::task::spawn(async move {
                let _permit = sem.acquire().await.unwrap();
                let path = std::path::PathBuf::from(&file_path);
                let fc = fields_clone;
                tokio::task::spawn_blocking(move || {
                    tags::read_file_tags(&path, fc.as_deref(), include_cover_art)
                })
                .await
                .unwrap_or_else(|e| tags::FileReadResult::Error {
                    path: file_path,
                    error: format!("task join error: {e}"),
                })
            }));
        }

        let mut results = Vec::with_capacity(inline_errors.len() + handles.len());
        let mut files_read: usize = 0;
        let mut files_failed: usize = inline_errors.len();
        let mut format_counts: HashMap<String, usize> = HashMap::new();

        // Prepend any inline errors from track_id resolution
        results.append(&mut inline_errors);

        for handle in handles {
            let result = handle.await.map_err(|e| internal(format!("join error: {e}")))?;
            match &result {
                tags::FileReadResult::Single { format, .. } => {
                    files_read += 1;
                    *format_counts.entry(format.clone()).or_insert(0) += 1;
                }
                tags::FileReadResult::Wav { format, .. } => {
                    files_read += 1;
                    *format_counts.entry(format.clone()).or_insert(0) += 1;
                }
                tags::FileReadResult::Error { .. } => {
                    files_failed += 1;
                }
            }
            results.push(result);
        }

        let output = serde_json::json!({
            "summary": {
                "files_read": files_read,
                "files_failed": files_failed,
                "formats": format_counts,
            },
            "results": results,
        });

        let json = serde_json::to_string_pretty(&output).map_err(|e| internal(format!("{e}")))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        description = "Write metadata tags to audio files on disk. Supports setting and deleting individual fields. Use dry_run to preview changes before writing."
    )]
    async fn write_file_tags(
        &self,
        params: Parameters<WriteFileTagsParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let dry_run = p.dry_run.unwrap_or(false);

        // Build WriteEntry values (per-entry validation happens in tags::write_file_tags)
        let entries: Vec<tags::WriteEntry> = p
            .writes
            .into_iter()
            .map(|e| tags::WriteEntry {
                path: PathBuf::from(&e.path),
                tags: e.tags,
                wav_targets: e.wav_targets.unwrap_or_else(|| {
                    vec![tags::WavTarget::Id3v2, tags::WavTarget::RiffInfo]
                }),
            })
            .collect();

        if dry_run {
            // Dry-run: can be parallel since it only reads
            let mut results = Vec::with_capacity(entries.len());
            for entry in entries {
                let result = tokio::task::spawn_blocking(move || {
                    tags::write_file_tags_dry_run(&entry)
                })
                .await
                .map_err(|e| internal(format!("join error: {e}")))?;
                results.push(result);
            }

            let mut previewed: usize = 0;
            let mut failed: usize = 0;
            for r in &results {
                match r {
                    tags::FileDryRunResult::Preview { .. } => previewed += 1,
                    tags::FileDryRunResult::Error { .. } => failed += 1,
                }
            }

            let output = serde_json::json!({
                "dry_run": true,
                "summary": {
                    "files_previewed": previewed,
                    "files_failed": failed,
                },
                "results": results,
            });

            let json = serde_json::to_string_pretty(&output).map_err(|e| internal(format!("{e}")))?;
            Ok(CallToolResult::success(vec![Content::text(json)]))
        } else {
            // Actual writes: sequential
            let mut results = Vec::with_capacity(entries.len());
            let mut files_written: usize = 0;
            let mut files_failed: usize = 0;
            let mut total_fields_written: usize = 0;

            for entry in entries {
                let result = tokio::task::spawn_blocking(move || {
                    tags::write_file_tags(&entry)
                })
                .await
                .map_err(|e| internal(format!("join error: {e}")))?;

                match &result {
                    tags::FileWriteResult::Ok { fields_written, .. } => {
                        files_written += 1;
                        total_fields_written += fields_written.len();
                    }
                    tags::FileWriteResult::Error { .. } => files_failed += 1,
                }
                results.push(result);
            }

            let output = serde_json::json!({
                "summary": {
                    "files_written": files_written,
                    "files_failed": files_failed,
                    "fields_written": total_fields_written,
                },
                "results": results,
            });

            let json = serde_json::to_string_pretty(&output).map_err(|e| internal(format!("{e}")))?;
            Ok(CallToolResult::success(vec![Content::text(json)]))
        }
    }

    #[tool(
        description = "Extract cover art from an audio file and save to disk."
    )]
    async fn extract_cover_art(
        &self,
        params: Parameters<ExtractCoverArtParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let path = PathBuf::from(&p.path);
        let output_path = p.output_path.map(PathBuf::from);
        let picture_type = p.picture_type.unwrap_or_else(|| "front_cover".to_string());

        let result = tokio::task::spawn_blocking(move || {
            tags::extract_cover_art(
                &path,
                output_path.as_deref(),
                &picture_type,
            )
        })
        .await
        .map_err(|e| internal(format!("join error: {e}")))?
        .map_err(|e| internal(e.to_string()))?;

        let json = serde_json::to_string_pretty(&result).map_err(|e| internal(format!("{e}")))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        description = "Embed cover art into one or more audio files."
    )]
    async fn embed_cover_art(
        &self,
        params: Parameters<EmbedCoverArtParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let image_path = PathBuf::from(&p.image_path);
        let picture_type = p.picture_type.unwrap_or_else(|| "front_cover".to_string());

        // Read image metadata before the embed loop
        let image_size_bytes = std::fs::metadata(&image_path)
            .map(|m| m.len() as usize)
            .unwrap_or(0);
        let image_format = {
            let data = std::fs::read(&image_path)
                .map_err(|e| internal(format!("Failed to read image: {e}")))?;
            if data.starts_with(&[0xff, 0xd8]) {
                "jpeg"
            } else if data.starts_with(&[0x89, 0x50, 0x4e, 0x47]) {
                "png"
            } else if data.starts_with(&[0x47, 0x49, 0x46]) {
                "gif"
            } else if data.starts_with(&[0x42, 0x4d]) {
                "bmp"
            } else {
                "unknown"
            }
        };

        let mut results = Vec::with_capacity(p.targets.len());
        let mut files_embedded: usize = 0;
        let mut files_failed: usize = 0;

        // Sequential writes to avoid file contention
        for target in p.targets {
            let img = image_path.clone();
            let tgt = PathBuf::from(&target);
            let pt = picture_type.clone();
            let result = tokio::task::spawn_blocking(move || {
                tags::embed_cover_art(&img, &tgt, &pt)
            })
            .await
            .map_err(|e| internal(format!("join error: {e}")))?;

            match &result {
                tags::FileEmbedResult::Ok { .. } => files_embedded += 1,
                tags::FileEmbedResult::Error { .. } => files_failed += 1,
            }
            results.push(result);
        }

        let output = serde_json::json!({
            "summary": {
                "files_embedded": files_embedded,
                "files_failed": files_failed,
                "image_format": image_format,
                "image_size_bytes": image_size_bytes,
            },
            "results": results,
        });

        let json = serde_json::to_string_pretty(&output).map_err(|e| internal(format!("{e}")))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    // -----------------------------------------------------------------------
    // Audit engine
    // -----------------------------------------------------------------------

    #[tool(
        description = "Collection audit engine. Scan files for convention violations, query/resolve issues, and get summaries. Operations: scan, query_issues, resolve_issues, get_summary."
    )]
    async fn audit_state(
        &self,
        params: Parameters<AuditStateParams>,
    ) -> Result<CallToolResult, McpError> {
        let store_path = self.internal_store_path();

        match params.0 {
            AuditStateParams::Scan {
                scope,
                revalidate,
                skip_issue_types,
            } => {
                let revalidate = revalidate.unwrap_or(false);
                let skip: HashSet<audit::IssueType> = skip_issue_types
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|s| s.parse::<audit::IssueType>().ok())
                    .collect();

                let summary = tokio::task::spawn_blocking(move || {
                    let conn = store::open(&store_path)
                        .map_err(|e| format!("Failed to open internal store: {e}"))?;
                    audit::scan(&conn, &scope, revalidate, &skip)
                })
                .await
                .map_err(|e| internal(format!("join error: {e}")))?
                .map_err(internal)?;

                let json =
                    serde_json::to_string_pretty(&summary).map_err(|e| internal(format!("{e}")))?;
                Ok(CallToolResult::success(vec![Content::text(json)]))
            }

            AuditStateParams::QueryIssues {
                scope,
                status,
                issue_type,
                limit,
                offset,
            } => {
                let limit = limit.unwrap_or(100);
                let offset = offset.unwrap_or(0);

                let issues = tokio::task::spawn_blocking(move || {
                    let conn = store::open(&store_path)
                        .map_err(|e| format!("Failed to open internal store: {e}"))?;
                    audit::query_issues(
                        &conn,
                        &scope,
                        status.as_deref(),
                        issue_type.as_deref(),
                        limit,
                        offset,
                    )
                })
                .await
                .map_err(|e| internal(format!("join error: {e}")))?
                .map_err(internal)?;

                let json =
                    serde_json::to_string_pretty(&issues).map_err(|e| internal(format!("{e}")))?;
                Ok(CallToolResult::success(vec![Content::text(json)]))
            }

            AuditStateParams::ResolveIssues {
                issue_ids,
                resolution,
                note,
            } => {
                let count = tokio::task::spawn_blocking(move || {
                    let conn = store::open(&store_path)
                        .map_err(|e| format!("Failed to open internal store: {e}"))?;
                    audit::resolve_issues(
                        &conn,
                        &issue_ids,
                        &resolution,
                        note.as_deref(),
                    )
                })
                .await
                .map_err(|e| internal(format!("join error: {e}")))?
                .map_err(internal)?;

                let json = serde_json::json!({ "resolved": count });
                let text =
                    serde_json::to_string_pretty(&json).map_err(|e| internal(format!("{e}")))?;
                Ok(CallToolResult::success(vec![Content::text(text)]))
            }

            AuditStateParams::GetSummary { scope } => {
                let summary = tokio::task::spawn_blocking(move || {
                    let conn = store::open(&store_path)
                        .map_err(|e| format!("Failed to open internal store: {e}"))?;
                    audit::get_summary(&conn, &scope)
                })
                .await
                .map_err(|e| internal(format!("join error: {e}")))?
                .map_err(internal)?;

                let json =
                    serde_json::to_string_pretty(&summary).map_err(|e| internal(format!("{e}")))?;
                Ok(CallToolResult::success(vec![Content::text(json)]))
            }
        }
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

    let (stratum_json, stratum_parse_error) = match stratum_cache {
        Some(sc) => match serde_json::from_str::<serde_json::Value>(&sc.features_json) {
            Ok(val) => (Some(val), None),
            Err(e) => (None, Some(format!("stratum-dsp cache JSON corrupt: {e}"))),
        },
        None => (None, None),
    };
    let (essentia_data, essentia_parse_error) = match essentia_cache {
        Some(ec) => match serde_json::from_str::<audio::EssentiaOutput>(&ec.features_json) {
            Ok(val) => (Some(val), None),
            Err(e) => (None, Some(format!("essentia cache JSON corrupt: {e}"))),
        },
        None => (None, None),
    };
    let essentia_json = essentia_data.as_ref()
        .and_then(|e| serde_json::to_value(e).ok());

    let (bpm_agreement, key_agreement) = if let Some(ref sj) = stratum_json {
        let stratum_bpm = sj.get("bpm").and_then(|v| v.as_f64());
        let stratum_key = sj.get("key").and_then(|v| v.as_str());

        let bpm_agree = stratum_bpm.map(|sb| (sb - track.bpm).abs() <= 2.0);
        let key_agree = stratum_key.map(|sk| sk.eq_ignore_ascii_case(&track.key));

        (bpm_agree, key_agree)
    } else {
        (None, None)
    };

    let has_analysis = stratum_json.is_some() || essentia_json.is_some()
        || stratum_parse_error.is_some() || essentia_parse_error.is_some();
    let audio_analysis = if has_analysis {
        let mut obj = serde_json::json!({
            "stratum_dsp": stratum_json,
            "essentia": essentia_json,
            "bpm_agreement": bpm_agreement,
            "key_agreement": key_agreement,
        });
        if let Some(err) = &stratum_parse_error {
            obj["stratum_dsp_parse_error"] = serde_json::json!(err);
        }
        if let Some(err) = &essentia_parse_error {
            obj["essentia_parse_error"] = serde_json::json!(err);
        }
        obj
    } else {
        serde_json::Value::Null
    };

    let discogs_val = parse_enrichment_cache(discogs_cache);
    let beatport_val = parse_enrichment_cache(beatport_cache);

    let staged_val = staged.map(|s| {
        serde_json::json!({
            "genre": s.genre,
            "comments": s.comments,
            "rating": s.rating,
            "color": s.color,
        })
    });

    let data_completeness = serde_json::json!({
        "rekordbox": true,
        "stratum_dsp": stratum_cache.is_some(),
        "essentia": essentia_cache.is_some(),
        "essentia_installed": essentia_installed,
        "discogs": discogs_cache.is_some(),
        "beatport": beatport_cache.is_some(),
    });

    let current_genre_canonical = if track.genre.is_empty() {
        serde_json::Value::Null
    } else if let Some(canonical) = genre::canonical_casing(&track.genre) {
        serde_json::json!(canonical)
    } else if let Some(canonical) = genre::normalize_genre(&track.genre) {
        serde_json::json!(canonical)
    } else {
        serde_json::Value::Null
    };

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
