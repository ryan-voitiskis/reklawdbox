use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use rmcp::handler::server::tool::ToolRouter;
use rmcp::model::{CallToolResult, Content, ServerCapabilities, ServerInfo};
use rmcp::{ErrorData as McpError, ServerHandler, tool, tool_handler, tool_router};
use rusqlite::Connection;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::audit;
use crate::audio;
use crate::beatport;
use crate::changes::ChangeManager;
use crate::color;
use crate::corpus;
use crate::db;
use crate::discogs;
use crate::genre;
use crate::store;
use crate::tags;
use crate::types::TrackChange;
use crate::xml;

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

        if let Some(cfg) = discogs::BrokerConfig::from_env() {
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

        beatport::lookup(&self.state.http, artist, title).await
    }
}

fn err(msg: String) -> McpError {
    McpError::internal_error(msg, None)
}

#[cfg(test)]
type DiscogsLookupOverrideResult = Result<Option<discogs::DiscogsResult>, discogs::LookupError>;
#[cfg(test)]
type BeatportLookupOverrideResult = Result<Option<beatport::BeatportResult>, String>;

#[cfg(test)]
type DiscogsLookupOverrideKey = (String, String, Option<String>);
#[cfg(test)]
type BeatportLookupOverrideKey = (String, String);

#[cfg(test)]
static TEST_DISCOGS_LOOKUP_OVERRIDES: OnceLock<
    Mutex<HashMap<DiscogsLookupOverrideKey, DiscogsLookupOverrideResult>>,
> = OnceLock::new();
#[cfg(test)]
static TEST_BEATPORT_LOOKUP_OVERRIDES: OnceLock<
    Mutex<HashMap<BeatportLookupOverrideKey, BeatportLookupOverrideResult>>,
> = OnceLock::new();

#[cfg(test)]
fn set_test_discogs_lookup_override(
    artist: &str,
    title: &str,
    album: Option<&str>,
    result: DiscogsLookupOverrideResult,
) {
    let map = TEST_DISCOGS_LOOKUP_OVERRIDES.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(mut guard) = map.lock() {
        guard.insert(
            (
                artist.to_string(),
                title.to_string(),
                album.map(str::to_string),
            ),
            result,
        );
    }
}

#[cfg(test)]
fn take_test_discogs_lookup_override(
    artist: &str,
    title: &str,
    album: Option<&str>,
) -> Option<DiscogsLookupOverrideResult> {
    let map = TEST_DISCOGS_LOOKUP_OVERRIDES.get_or_init(|| Mutex::new(HashMap::new()));
    map.lock().ok()?.remove(&(
        artist.to_string(),
        title.to_string(),
        album.map(str::to_string),
    ))
}

#[cfg(test)]
fn set_test_beatport_lookup_override(
    artist: &str,
    title: &str,
    result: BeatportLookupOverrideResult,
) {
    let map = TEST_BEATPORT_LOOKUP_OVERRIDES.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(mut guard) = map.lock() {
        guard.insert((artist.to_string(), title.to_string()), result);
    }
}

#[cfg(test)]
fn take_test_beatport_lookup_override(
    artist: &str,
    title: &str,
) -> Option<BeatportLookupOverrideResult> {
    let map = TEST_BEATPORT_LOOKUP_OVERRIDES.get_or_init(|| Mutex::new(HashMap::new()));
    map.lock()
        .ok()?
        .remove(&(artist.to_string(), title.to_string()))
}

fn lookup_output_with_cache_metadata(
    payload: serde_json::Value,
    cache_hit: bool,
    cached_at: Option<&str>,
) -> serde_json::Value {
    match payload {
        serde_json::Value::Object(mut map) => {
            map.insert("cache_hit".to_string(), serde_json::json!(cache_hit));
            if let Some(cached_at) = cached_at {
                map.insert("cached_at".to_string(), serde_json::json!(cached_at));
            }
            serde_json::Value::Object(map)
        }
        other => {
            let mut map = serde_json::Map::new();
            map.insert("result".to_string(), other);
            map.insert("cache_hit".to_string(), serde_json::json!(cache_hit));
            if let Some(cached_at) = cached_at {
                map.insert("cached_at".to_string(), serde_json::json!(cached_at));
            }
            serde_json::Value::Object(map)
        }
    }
}

fn auth_remediation_message(remediation: &discogs::AuthRemediation) -> String {
    let mut lines = vec![remediation.message.clone()];
    if let Some(auth_url) = remediation.auth_url.as_deref() {
        lines.push(format!("Open this URL in a browser: {auth_url}"));
    }
    if let Some(poll_interval) = remediation.poll_interval_seconds {
        lines.push(format!("Suggested poll interval: {poll_interval}s"));
    }
    if let Some(expires_at) = remediation.expires_at {
        lines.push(format!("Auth session expires_at (unix): {expires_at}"));
    }
    lines.join("\n")
}

const ESSENTIA_PYTHON_ENV_VAR: &str = "CRATE_DIG_ESSENTIA_PYTHON";
const ESSENTIA_IMPORT_CHECK_SCRIPT: &str = "import essentia; print(essentia.__version__)";
const ESSENTIA_PROBE_TIMEOUT_SECS: u64 = 5;

fn validate_essentia_python(path: &str) -> bool {
    validate_essentia_python_with_timeout(path, Duration::from_secs(ESSENTIA_PROBE_TIMEOUT_SECS))
}

fn validate_essentia_python_with_timeout(path: &str, timeout: Duration) -> bool {
    let mut child = match std::process::Command::new(path)
        .args(["-c", ESSENTIA_IMPORT_CHECK_SCRIPT])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return false,
    };

    let Some(mut stdout_pipe) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return false;
    };
    let Some(mut stderr_pipe) = child.stderr.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return false;
    };

    let stdout_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout_pipe.read_to_end(&mut buf);
        buf
    });
    let stderr_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr_pipe.read_to_end(&mut buf);
        buf
    });

    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };

    let stdout = stdout_handle.join().unwrap_or_default();
    let _stderr = stderr_handle.join().unwrap_or_default();

    let Some(status) = status else {
        return false;
    };
    if !status.success() {
        return false;
    }

    let stdout = String::from_utf8_lossy(&stdout);
    let version_line = stdout.lines().map(str::trim).find(|line| !line.is_empty());
    matches!(
        version_line,
        Some(line) if line.chars().any(|ch| ch.is_ascii_digit())
    )
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

pub(crate) fn probe_essentia_python_path() -> Option<String> {
    let env_override = std::env::var(ESSENTIA_PYTHON_ENV_VAR).ok();
    let default_candidate =
        dirs::home_dir().map(|home| home.join(".local/share/reklawdbox/essentia-venv/bin/python"));
    probe_essentia_python_from_sources(env_override.as_deref(), default_candidate)
}

const ESSENTIA_VENV_RELPATH: &str = ".local/share/reklawdbox/essentia-venv";

fn essentia_venv_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(ESSENTIA_VENV_RELPATH))
}

fn essentia_setup_hint() -> String {
    let mut checked = Vec::new();

    match std::env::var(ESSENTIA_PYTHON_ENV_VAR) {
        Ok(val) if !val.trim().is_empty() => {
            checked.push(format!(
                "env {ESSENTIA_PYTHON_ENV_VAR}={val} (not a valid Essentia Python)"
            ));
        }
        _ => {
            checked.push(format!("env {ESSENTIA_PYTHON_ENV_VAR} (not set)"));
        }
    }

    if let Some(venv_dir) = essentia_venv_dir() {
        let python_path = venv_dir.join("bin/python");
        if python_path.exists() {
            checked.push(format!(
                "{} (exists but Essentia import failed)",
                python_path.display()
            ));
        } else {
            checked.push(format!("{} (not found)", python_path.display()));
        }
    }

    format!(
        "Essentia not found. Checked: {}. Call the setup_essentia tool to install automatically.",
        checked.join(", ")
    )
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

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct SearchFilterParams {
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
    #[schemars(description = "Filter by whether track has a genre set")]
    pub has_genre: Option<bool>,
    #[schemars(description = "Filter by label name (partial match)")]
    pub label: Option<String>,
    #[schemars(description = "Filter by file path/folder (partial match)")]
    pub path: Option<String>,
    #[schemars(
        description = "Only tracks added on or after this date (ISO date, e.g. '2026-01-01')"
    )]
    pub added_after: Option<String>,
    #[schemars(
        description = "Only tracks added on or before this date (ISO date, e.g. '2026-12-31')"
    )]
    pub added_before: Option<String>,
}

impl SearchFilterParams {
    fn into_search_params(
        self,
        exclude_samples: bool,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> db::SearchParams {
        db::SearchParams {
            query: self.query,
            artist: self.artist,
            genre: self.genre,
            rating_min: self.rating_min,
            bpm_min: self.bpm_min,
            bpm_max: self.bpm_max,
            key: self.key,
            playlist: None,
            has_genre: self.has_genre,
            label: self.label,
            path: self.path,
            added_after: self.added_after,
            added_before: self.added_before,
            exclude_samples,
            limit,
            offset,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchTracksParams {
    #[serde(flatten)]
    pub filters: SearchFilterParams,
    #[schemars(description = "Filter by playlist ID")]
    pub playlist: Option<String>,
    #[schemars(description = "Include Rekordbox factory samples (default false)")]
    pub include_samples: Option<bool>,
    #[schemars(description = "Max results (default 50, max 200)")]
    pub limit: Option<u32>,
    #[schemars(description = "Offset for pagination (skip first N results)")]
    pub offset: Option<u32>,
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
pub struct WriteXmlPlaylistInput {
    #[schemars(description = "Playlist name")]
    pub name: String,
    #[schemars(description = "Track IDs in playlist order")]
    pub track_ids: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WriteXmlParams {
    #[schemars(
        description = "Output file path (default: ./rekordbox-exports/reklawdbox-{timestamp}.xml)"
    )]
    pub output_path: Option<String>,
    #[schemars(
        description = "Optional playlist exports. Each playlist includes a name and ordered track_ids."
    )]
    pub playlists: Option<Vec<WriteXmlPlaylistInput>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PreviewChangesParams {
    #[schemars(description = "Filter to specific track IDs (if empty, shows all staged changes)")]
    pub track_ids: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ClearChangesParams {
    #[schemars(description = "Track IDs to clear (if empty, clears all)")]
    pub track_ids: Option<Vec<String>>,
    #[schemars(
        description = "Specific fields to unstage: \"genre\", \"comments\", \"rating\", \"color\". If omitted, clears all fields (removes entire entries)."
    )]
    pub fields: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SuggestNormalizationsParams {
    #[schemars(description = "Only show genres with at least this many tracks (default 1)")]
    pub min_count: Option<i32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LookupDiscogsParams {
    #[schemars(description = "Track ID — auto-fills artist/title/album from library")]
    pub track_id: Option<String>,
    #[schemars(description = "Artist name (required if no track_id)")]
    pub artist: Option<String>,
    #[schemars(description = "Track title (required if no track_id)")]
    pub title: Option<String>,
    #[schemars(description = "Album/release title for more accurate matching")]
    pub album: Option<String>,
    #[schemars(description = "Bypass cache and fetch fresh data (default false)")]
    pub force_refresh: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LookupBeatportParams {
    #[schemars(description = "Track ID — auto-fills artist/title from library")]
    pub track_id: Option<String>,
    #[schemars(description = "Artist name (required if no track_id)")]
    pub artist: Option<String>,
    #[schemars(description = "Track title (required if no track_id)")]
    pub title: Option<String>,
    #[schemars(description = "Bypass cache and fetch fresh data (default false)")]
    pub force_refresh: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct EnrichTracksParams {
    #[serde(flatten)]
    pub filters: SearchFilterParams,
    #[schemars(description = "Specific track IDs to enrich (highest priority selector)")]
    pub track_ids: Option<Vec<String>>,
    #[schemars(description = "Enrich tracks in this playlist")]
    pub playlist_id: Option<String>,
    #[schemars(description = "Max tracks to enrich (default 50)")]
    pub max_tracks: Option<u32>,
    #[schemars(description = "Providers to use: 'discogs', 'beatport' (default ['discogs'])")]
    pub providers: Option<Vec<crate::types::Provider>>,
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
    #[serde(flatten)]
    pub filters: SearchFilterParams,
    #[schemars(description = "Specific track IDs to analyze (highest priority selector)")]
    pub track_ids: Option<Vec<String>>,
    #[schemars(description = "Analyze tracks in this playlist")]
    pub playlist_id: Option<String>,
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
    #[serde(flatten)]
    pub filters: SearchFilterParams,
    #[schemars(description = "Specific track IDs to resolve (highest priority selector)")]
    pub track_ids: Option<Vec<String>>,
    #[schemars(description = "Resolve tracks in this playlist")]
    pub playlist_id: Option<String>,
    #[schemars(description = "Max tracks to resolve (default 50)")]
    pub max_tracks: Option<u32>,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SetPriority {
    Balanced,
    Harmonic,
    Energy,
    Genre,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EnergyPhase {
    Warmup,
    Build,
    Peak,
    Release,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EnergyCurvePreset {
    WarmupBuildPeakRelease,
    Flat,
    PeakOnly,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum EnergyCurveInput {
    Preset(EnergyCurvePreset),
    Custom(Vec<EnergyPhase>),
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BuildSetParams {
    #[schemars(description = "Pool of candidate track IDs (pre-filtered by agent)")]
    pub track_ids: Vec<String>,
    #[schemars(description = "Desired number of tracks in each candidate set")]
    pub target_tracks: u32,
    #[schemars(description = "Weighting axis (balanced, harmonic, energy, genre)")]
    pub priority: Option<SetPriority>,
    #[schemars(
        description = "Energy curve: preset name ('warmup_build_peak_release', 'flat', 'peak_only') or an array of phase strings (warmup/build/peak/release), one per target position."
    )]
    pub energy_curve: Option<EnergyCurveInput>,
    #[schemars(description = "Optional track ID to force as the opening track")]
    pub start_track_id: Option<String>,
    #[schemars(description = "Number of set candidates to generate (default 2, max 3)")]
    pub candidates: Option<u8>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScoreTransitionParams {
    #[schemars(description = "Source track ID")]
    pub from_track_id: String,
    #[schemars(description = "Destination track ID")]
    pub to_track_id: String,
    #[schemars(description = "Energy phase preference (warmup, build, peak, release)")]
    pub energy_phase: Option<EnergyPhase>,
    #[schemars(description = "Weighting axis (balanced, harmonic, energy, genre)")]
    pub priority: Option<SetPriority>,
}

// ---------------------------------------------------------------------------
// Native tag tool params
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
struct ReadFileTagsParams {
    #[schemars(description = "Explicit file paths to read")]
    paths: Option<Vec<String>>,

    #[schemars(description = "Resolve file paths from Rekordbox track IDs")]
    track_ids: Option<Vec<String>>,

    #[schemars(description = "Scan directory for audio files")]
    directory: Option<String>,

    #[schemars(
        description = "Glob filter within directory (default: all audio files). Only used with directory."
    )]
    glob: Option<String>,

    #[schemars(description = "Scan subdirectories (default: false). Only used with directory.")]
    recursive: Option<bool>,

    #[schemars(
        description = "Return only these fields (default: all). Valid: artist, title, album, album_artist, genre, year, track, disc, comment, publisher, bpm, key, composer, remixer"
    )]
    fields: Option<Vec<String>>,

    #[schemars(description = "Include cover art metadata (default: false)")]
    include_cover_art: Option<bool>,

    #[schemars(description = "Max files to read (default: 200, max: 2000)")]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct WriteFileTagsParams {
    #[schemars(description = "Array of write operations")]
    writes: Vec<WriteFileTagsEntry>,

    #[schemars(description = "Preview changes without writing (default: false)")]
    dry_run: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct WriteFileTagsEntry {
    #[schemars(description = "Path to the audio file")]
    path: String,

    #[schemars(
        description = "Tag fields to write. Keys are field names, values are strings to set or null to delete."
    )]
    tags: HashMap<String, Option<String>>,

    #[schemars(
        description = "WAV only: which tag layers to write (default: both). Values: \"id3v2\", \"riff_info\""
    )]
    wav_targets: Option<Vec<tags::WavTarget>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ExtractCoverArtParams {
    #[schemars(description = "Path to the audio file")]
    path: String,

    #[schemars(
        description = "Where to save the extracted art (default: cover.{ext} in same directory)"
    )]
    output_path: Option<String>,

    #[schemars(description = "Which art to extract (default: front_cover)")]
    picture_type: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct EmbedCoverArtParams {
    #[schemars(description = "Path to the image file")]
    image_path: String,

    #[schemars(description = "Audio files to embed art into")]
    targets: Vec<String>,

    #[schemars(description = "Picture type (default: front_cover)")]
    picture_type: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(tag = "operation")]
enum AuditStateParams {
    #[serde(rename = "scan")]
    Scan {
        #[schemars(description = "Directory path to audit (trailing / enforced)")]
        scope: String,

        #[schemars(description = "Re-read all files including unchanged (default: false)")]
        revalidate: Option<bool>,

        #[schemars(
            description = "Issue types to exclude from detection (e.g. [\"GENRE_SET\"])"
        )]
        skip_issue_types: Option<Vec<String>>,
    },

    #[serde(rename = "query_issues")]
    QueryIssues {
        #[schemars(description = "Directory path prefix to filter issues")]
        scope: String,

        #[schemars(description = "Filter by status: open | resolved | accepted | deferred")]
        status: Option<String>,

        #[schemars(description = "Filter by issue type (e.g. WAV_TAG3_MISSING)")]
        issue_type: Option<String>,

        #[schemars(description = "Max results (default: 100)")]
        limit: Option<u32>,

        #[schemars(description = "Offset for pagination (default: 0)")]
        offset: Option<u32>,
    },

    #[serde(rename = "resolve_issues")]
    ResolveIssues {
        #[schemars(description = "Issue IDs to resolve")]
        issue_ids: Vec<i64>,

        #[schemars(description = "Resolution: accepted_as_is | wont_fix | deferred")]
        resolution: String,

        #[schemars(description = "Optional user comment")]
        note: Option<String>,
    },

    #[serde(rename = "get_summary")]
    GetSummary {
        #[schemars(description = "Directory path prefix for summary")]
        scope: String,
    },
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
                let tracks = db::get_tracks_by_exact_genre(&conn, &gc.name, true)
                    .map_err(|e| err(format!("DB error: {e}")))?;
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
                    .map_err(|e| err(format!("DB error: {e}")))?;
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
        let json = serde_json::to_string_pretty(&result).map_err(|e| err(format!("{e}")))?;
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
            let json = serde_json::to_string_pretty(&result).map_err(|e| err(format!("{e}")))?;
            return Ok(CallToolResult::success(vec![Content::text(json)]));
        }

        let backup_script = ["scripts/backup.sh", "backup.sh"]
            .iter()
            .find(|path| std::path::Path::new(path).exists());
        if let Some(script_path) = backup_script {
            eprintln!("[reklawdbox] Running pre-op backup...");
            let output = std::process::Command::new("bash")
                .arg(script_path)
                .arg("--pre-op")
                .output();
            match output {
                Ok(o) if o.status.success() => eprintln!("[reklawdbox] Backup completed."),
                Ok(o) => {
                    let stderr_out = String::from_utf8_lossy(&o.stderr);
                    eprintln!("[reklawdbox] Backup warning: {stderr_out}");
                }
                Err(e) => eprintln!("[reklawdbox] Backup skipped: {e}"),
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
                return Err(err(format!("DB error: {e}")));
            }
        };
        let mut tracks_by_id: HashMap<String, crate::types::Track> = current_tracks
            .into_iter()
            .map(|track| (track.id.clone(), track))
            .collect();
        let missing_ids: Vec<String> = ids
            .iter()
            .filter(|id| !tracks_by_id.contains_key(id.as_str()))
            .cloned()
            .collect();
        if !missing_ids.is_empty() {
            self.state.changes.restore(snapshot);
            return Err(err(format!(
                "Track IDs not found in database: {}",
                missing_ids.join(", ")
            )));
        }
        let ordered_tracks: Vec<crate::types::Track> = ids
            .iter()
            .filter_map(|id| tracks_by_id.remove(id))
            .collect();
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
            return Err(err(format!("Write error: {e}")));
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
        let json = serde_json::to_string_pretty(&result).map_err(|e| err(format!("{e}")))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Clear staged changes for specific tracks or all")]
    async fn clear_changes(
        &self,
        params: Parameters<ClearChangesParams>,
    ) -> Result<CallToolResult, McpError> {
        const VALID_FIELDS: &[&str] = &["genre", "comments", "rating", "color"];

        if let Some(ref fields) = params.0.fields {
            for f in fields {
                if !VALID_FIELDS.contains(&f.as_str()) {
                    return Err(McpError::invalid_params(
                        format!(
                            "unknown field '{}'. Valid fields: {}",
                            f,
                            VALID_FIELDS.join(", ")
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
            let json = serde_json::to_string_pretty(&result).map_err(|e| err(format!("{e}")))?;
            Ok(CallToolResult::success(vec![Content::text(json)]))
        } else {
            let (cleared, remaining) = self.state.changes.clear(params.0.track_ids);
            let result = serde_json::json!({
                "cleared": cleared,
                "remaining": remaining,
            });
            let json = serde_json::to_string_pretty(&result).map_err(|e| err(format!("{e}")))?;
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
                .map_err(|e| err(format!("DB error: {e}")))?
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

        let norm_artist = discogs::normalize(&artist);
        let norm_title = discogs::normalize(&title);

        if !force_refresh {
            let store = self.internal_conn()?;
            if let Some(cached) =
                store::get_enrichment(&store, "discogs", &norm_artist, &norm_title)
                    .map_err(|e| err(format!("Cache read error: {e}")))?
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
                    serde_json::to_string_pretty(&result).map_err(|e| err(format!("{e}")))?;
                return Ok(CallToolResult::success(vec![Content::text(json)]));
            }
        }

        let result = self
            .lookup_discogs_live(&artist, &title, album.as_deref())
            .await
            .map_err(|e| match e.auth_remediation() {
                Some(remediation) => err(auth_remediation_message(remediation)),
                None => err(format!("Discogs error: {e}")),
            })?;

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

        let output = lookup_output_with_cache_metadata(
            serde_json::to_value(&result).map_err(|e| err(format!("{e}")))?,
            false,
            None,
        );
        let json = serde_json::to_string_pretty(&output).map_err(|e| err(format!("{e}")))?;
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
                .map_err(|e| err(format!("DB error: {e}")))?
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

        let norm_artist = discogs::normalize(&artist);
        let norm_title = discogs::normalize(&title);

        if !force_refresh {
            let store = self.internal_conn()?;
            if let Some(cached) =
                store::get_enrichment(&store, "beatport", &norm_artist, &norm_title)
                    .map_err(|e| err(format!("Cache read error: {e}")))?
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
                    serde_json::to_string_pretty(&result).map_err(|e| err(format!("{e}")))?;
                return Ok(CallToolResult::success(vec![Content::text(json)]));
            }
        }

        let result = self
            .lookup_beatport_live(&artist, &title)
            .await
            .map_err(|e| err(format!("Beatport error: {e}")))?;

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

        let output = lookup_output_with_cache_metadata(
            serde_json::to_value(&result).map_err(|e| err(format!("{e}")))?,
            false,
            None,
        );
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
            let norm_artist = discogs::normalize(&track.artist);
            let norm_title = discogs::normalize(&track.title);

            for provider in &providers {
                if skip_cached && !force_refresh {
                    let store = self.internal_conn()?;
                    if store::get_enrichment(
                        &store,
                        provider.as_str(),
                        &norm_artist,
                        &norm_title,
                    )
                    .map_err(|e| err(format!("Cache read error: {e}")))?
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
                                    serde_json::to_string(&r).map_err(|e| err(format!("{e}")))?;
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
                                .map_err(|e| err(format!("Cache write error: {e}")))?;
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
                                .map_err(|e| err(format!("Cache write error: {e}")))?;
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
                                    serde_json::to_string(&r).map_err(|e| err(format!("{e}")))?;
                                let store = self.internal_conn()?;
                                store::set_enrichment(
                                    &store,
                                    provider.as_str(),
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
                                    provider.as_str(),
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
                                    "provider": provider.as_str(),
                                    "error": e,
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
                && cached_entry.file_size == file_size && cached_entry.file_mtime == file_mtime
            {
                stratum_dsp = Some(
                    serde_json::from_str(&cached_entry.features_json)
                        .map_err(|e| err(format!("Cache parse error: {e}")))?,
                );
                stratum_cache_hit = true;
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
                    && cached_entry.file_size == file_size && cached_entry.file_mtime == file_mtime
                {
                    essentia = Some(
                        serde_json::from_str(&cached_entry.features_json)
                            .map_err(|e| err(format!("Cache parse error: {e}")))?,
                    );
                    essentia_cache_hit = Some(true);
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
                    && cached_entry.file_size == file_size && cached_entry.file_mtime == file_mtime
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
                        && cached_entry.file_size == row.file_size
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
        let json = serde_json::to_string_pretty(&result).map_err(|e| err(format!("{e}")))?;
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
                    serde_json::to_string_pretty(&result).map_err(|e| err(format!("{e}")))?;
                return Ok(CallToolResult::success(vec![Content::text(json)]));
            }
            // Stale override — clear it and proceed with fresh install
            if let Ok(mut guard) = self.state.essentia_python_override.lock() {
                *guard = None;
            }
        }

        let venv_dir = essentia_venv_dir()
            .ok_or_else(|| err("Cannot determine home directory for venv location".to_string()))?;

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
                    err(format!(
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
            .map_err(|e| err(format!("venv task failed: {e}")))?
            .map_err(|e| err(format!("Failed to run {python_bin} -m venv: {e}")))?;

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
            .map_err(|e| err(format!("pip task failed: {e}")))?
            .map_err(|e| err(format!("Failed to run pip install: {e}")))?;

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
            .map_err(|e| err(format!("validate task failed: {e}")))?
            .map_err(|e| err(format!("Failed to validate essentia installation: {e}")))?;

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
                .map_err(|_| err("essentia override lock poisoned".to_string()))?;
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
            let json = serde_json::to_string_pretty(&result).map_err(|e| err(format!("{e}")))?;
            return Ok(CallToolResult::success(vec![Content::text(json)]));
        }

        Err(err(format!(
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
                .map_err(|e| err(format!("DB error: {e}")))?
                .ok_or_else(|| {
                    McpError::invalid_params(format!("Track '{}' not found", p.from_track_id), None)
                })?;
            let to = db::get_track(&conn, &p.to_track_id)
                .map_err(|e| err(format!("DB error: {e}")))?
                .ok_or_else(|| {
                    McpError::invalid_params(format!("Track '{}' not found", p.to_track_id), None)
                })?;
            (from, to)
        };

        let (from_profile, to_profile) = {
            let store = self.internal_conn()?;
            let from = build_track_profile(from_track, &store)
                .map_err(|e| err(format!("Failed to build source track profile: {e}")))?;
            let to = build_track_profile(to_track, &store)
                .map_err(|e| err(format!("Failed to build destination track profile: {e}")))?;
            (from, to)
        };

        let scores = score_transition_profiles(
            &from_profile,
            &to_profile,
            p.energy_phase,
            p.energy_phase,
            priority,
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

        let json = serde_json::to_string_pretty(&result).map_err(|e| err(format!("{e}")))?;
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
            db::get_tracks_by_ids(&conn, &deduped_ids).map_err(|e| err(format!("DB error: {e}")))?
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
                    .map_err(|e| err(format!("Failed to build track profile: {e}")))?;
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
            let sample_prefix = format!("{}%", db::escape_like(db::SAMPLER_PATH_PREFIX));
            let total_tracks: usize = conn
                .query_row(
                    "SELECT COUNT(*) FROM djmdContent
                     WHERE rb_local_deleted = 0
                       AND FolderPath NOT LIKE ?1 ESCAPE '\\'",
                    rusqlite::params![sample_prefix],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|e| err(format!("DB error: {e}")))?
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
                let norm_artist = discogs::normalize(&track.artist);
                let norm_title = discogs::normalize(&track.title);
                let audio_cache_key =
                    resolve_file_path(&track.file_path).unwrap_or_else(|_| track.file_path.clone());

                let has_discogs =
                    store::get_enrichment(&store, "discogs", &norm_artist, &norm_title)
                        .map_err(|e| err(format!("Cache read error: {e}")))?
                        .is_some();
                let has_beatport =
                    store::get_enrichment(&store, "beatport", &norm_artist, &norm_title)
                        .map_err(|e| err(format!("Cache read error: {e}")))?
                        .is_some();
                let has_stratum =
                    store::get_audio_analysis(&store, &audio_cache_key, "stratum-dsp")
                        .map_err(|e| err(format!("Cache read error: {e}")))?
                        .is_some();
                let has_essentia = store::get_audio_analysis(&store, &audio_cache_key, "essentia")
                    .map_err(|e| err(format!("Cache read error: {e}")))?
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

        let json = serde_json::to_string_pretty(&result).map_err(|e| err(format!("{e}")))?;
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
                .map_err(err)?
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
            let result = handle.await.map_err(|e| err(format!("join error: {e}")))?;
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

        let json = serde_json::to_string_pretty(&output).map_err(|e| err(format!("{e}")))?;
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
                .map_err(|e| err(format!("join error: {e}")))?;
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

            let json = serde_json::to_string_pretty(&output).map_err(|e| err(format!("{e}")))?;
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
                .map_err(|e| err(format!("join error: {e}")))?;

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

            let json = serde_json::to_string_pretty(&output).map_err(|e| err(format!("{e}")))?;
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
        .map_err(|e| err(format!("join error: {e}")))?
        .map_err(err)?;

        let json = serde_json::to_string_pretty(&result).map_err(|e| err(format!("{e}")))?;
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
                .map_err(|e| err(format!("Failed to read image: {e}")))?;
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
            .map_err(|e| err(format!("join error: {e}")))?;

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

        let json = serde_json::to_string_pretty(&output).map_err(|e| err(format!("{e}")))?;
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
                .map_err(|e| err(format!("join error: {e}")))?
                .map_err(|e| err(e))?;

                let json =
                    serde_json::to_string_pretty(&summary).map_err(|e| err(format!("{e}")))?;
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
                .map_err(|e| err(format!("join error: {e}")))?
                .map_err(|e| err(e))?;

                let json =
                    serde_json::to_string_pretty(&issues).map_err(|e| err(format!("{e}")))?;
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
                .map_err(|e| err(format!("join error: {e}")))?
                .map_err(|e| err(e))?;

                let json = serde_json::json!({ "resolved": count });
                let text =
                    serde_json::to_string_pretty(&json).map_err(|e| err(format!("{e}")))?;
                Ok(CallToolResult::success(vec![Content::text(text)]))
            }

            AuditStateParams::GetSummary { scope } => {
                let summary = tokio::task::spawn_blocking(move || {
                    let conn = store::open(&store_path)
                        .map_err(|e| format!("Failed to open internal store: {e}"))?;
                    audit::get_summary(&conn, &scope)
                })
                .await
                .map_err(|e| err(format!("join error: {e}")))?
                .map_err(|e| err(e))?;

                let json =
                    serde_json::to_string_pretty(&summary).map_err(|e| err(format!("{e}")))?;
                Ok(CallToolResult::success(vec![Content::text(json)]))
            }
        }
    }
}

use crate::audio::AUDIO_EXTENSIONS;

/// Scan a directory for audio files, optionally recursive, with optional glob filter.
fn scan_audio_directory(
    dir: &str,
    recursive: bool,
    glob_pattern: Option<&str>,
) -> Result<Vec<String>, String> {
    let dir_path = std::path::Path::new(dir);
    if !dir_path.is_dir() {
        return Err(format!("Not a directory: {dir}"));
    }

    // Compile glob matcher if a pattern was provided
    let glob_matcher = match glob_pattern {
        Some(pattern) => {
            let glob = globset::GlobBuilder::new(pattern)
                .literal_separator(true)
                .case_insensitive(true)
                .build()
                .map_err(|e| format!("Invalid glob pattern \"{pattern}\": {e}"))?;
            Some(glob.compile_matcher())
        }
        None => None,
    };

    let mut files = Vec::new();
    let mut dirs_to_scan = vec![dir_path.to_path_buf()];

    while let Some(current_dir) = dirs_to_scan.pop() {
        let entries = std::fs::read_dir(&current_dir)
            .map_err(|e| format!("Failed to read directory {}: {e}", current_dir.display()))?;

        for entry in entries {
            let entry = entry.map_err(|e| format!("Directory entry error: {e}"))?;
            let path = entry.path();

            if path.is_dir() && recursive {
                dirs_to_scan.push(path);
                continue;
            }

            if !path.is_file() {
                continue;
            }

            // Must be an audio file regardless of glob
            let is_audio = path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| AUDIO_EXTENSIONS.contains(&e.to_lowercase().as_str()));
            if !is_audio {
                continue;
            }

            // Apply glob filter against the filename
            if let Some(ref matcher) = glob_matcher {
                let file_name = match path.file_name().and_then(|n| n.to_str()) {
                    Some(n) => n,
                    None => continue,
                };
                if !matcher.is_match(file_name) {
                    continue;
                }
            }

            files.push(path.display().to_string());
        }
    }

    files.sort();
    Ok(files)
}

struct ResolveTracksOpts {
    /// Default max_tracks when track_ids are absent and max_tracks param is None.
    /// When track_ids IS present and this is Some, defaults to ids.len().
    /// None = no auto-default (used by cache_coverage).
    default_max: Option<u32>,
    /// Hard cap on effective max. Some(200) for bounded tools, None for unbounded.
    cap: Option<u32>,
    /// Post-filter to exclude sampler tracks (used by cache_coverage).
    exclude_samplers: bool,
}

/// Resolve tracks using priority: track_ids > playlist_id > search filters.
///
/// Shared by `enrich_tracks`, `analyze_audio_batch`, `resolve_tracks_data`, and `cache_coverage`.
fn resolve_tracks(
    conn: &Connection,
    track_ids: Option<&[String]>,
    playlist_id: Option<&str>,
    filters: SearchFilterParams,
    max_tracks_param: Option<u32>,
    opts: &ResolveTracksOpts,
) -> Result<Vec<crate::types::Track>, McpError> {
    let effective_max: Option<usize> = match opts.default_max {
        Some(default_when_no_ids) => {
            let default = track_ids.map_or(default_when_no_ids, |ids| ids.len() as u32);
            let mut max = max_tracks_param.unwrap_or(default);
            if let Some(cap) = opts.cap {
                max = max.min(cap);
            }
            Some(max as usize)
        }
        None => max_tracks_param.map(|m| {
            if let Some(cap) = opts.cap {
                m.min(cap) as usize
            } else {
                m as usize
            }
        }),
    };

    let bounded = opts.cap.is_some();

    let tracks = if let Some(ids) = track_ids {
        db::get_tracks_by_ids(conn, ids).map_err(|e| err(format!("DB error: {e}")))?
    } else if let Some(pid) = playlist_id {
        let db_limit = if bounded { effective_max.map(|m| m as u32) } else { None };
        if bounded {
            db::get_playlist_tracks(conn, pid, db_limit)
                .map_err(|e| err(format!("DB error: {e}")))?
        } else {
            db::get_playlist_tracks_unbounded(conn, pid, db_limit)
                .map_err(|e| err(format!("DB error: {e}")))?
        }
    } else {
        let limit = effective_max.map(|m| m as u32);
        let search = filters.into_search_params(true, limit, None);
        if bounded {
            db::search_tracks(conn, &search).map_err(|e| err(format!("DB error: {e}")))?
        } else {
            db::search_tracks_unbounded(conn, &search)
                .map_err(|e| err(format!("DB error: {e}")))?
        }
    };

    let mut tracks: Vec<_> = if opts.exclude_samplers {
        tracks
            .into_iter()
            .filter(|t| !t.file_path.starts_with(db::SAMPLER_PATH_PREFIX))
            .collect()
    } else {
        tracks
    };

    if let Some(max) = effective_max {
        tracks.truncate(max);
    }

    Ok(tracks)
}

fn describe_resolve_scope(params: &ResolveTracksDataParams) -> String {
    if let Some(track_ids) = &params.track_ids {
        if let Some(max_tracks) = params.max_tracks {
            return format!(
                "track_ids ({}) [max_tracks = {max_tracks}]",
                track_ids.len()
            );
        }
        return format!("track_ids ({})", track_ids.len());
    }

    if let Some(playlist_id) = &params.playlist_id {
        if let Some(max_tracks) = params.max_tracks {
            return format!("playlist_id = \"{playlist_id}\", max_tracks = {max_tracks}");
        }
        return format!("playlist_id = \"{playlist_id}\"");
    }

    let mut filters: Vec<String> = Vec::new();
    if let Some(query) = &params.filters.query {
        filters.push(format!("query ~= \"{query}\""));
    }
    if let Some(artist) = &params.filters.artist {
        filters.push(format!("artist ~= \"{artist}\""));
    }
    if let Some(genre) = &params.filters.genre {
        filters.push(format!("genre ~= \"{genre}\""));
    }
    if let Some(has_genre) = params.filters.has_genre {
        filters.push(format!("has_genre = {has_genre}"));
    }
    if let Some(bpm_min) = params.filters.bpm_min {
        filters.push(format!("bpm_min = {bpm_min}"));
    }
    if let Some(bpm_max) = params.filters.bpm_max {
        filters.push(format!("bpm_max = {bpm_max}"));
    }
    if let Some(key) = &params.filters.key {
        filters.push(format!("key = \"{key}\""));
    }
    if let Some(rating_min) = params.filters.rating_min {
        filters.push(format!("rating_min = {rating_min}"));
    }
    if let Some(label) = &params.filters.label {
        filters.push(format!("label ~= \"{label}\""));
    }
    if let Some(path) = &params.filters.path {
        filters.push(format!("path ~= \"{path}\""));
    }
    if let Some(added_after) = &params.filters.added_after {
        filters.push(format!("added_after = \"{added_after}\""));
    }
    if let Some(added_before) = &params.filters.added_before {
        filters.push(format!("added_before = \"{added_before}\""));
    }
    if let Some(max_tracks) = params.max_tracks {
        filters.push(format!("max_tracks = {max_tracks}"));
    }

    if filters.is_empty() {
        "all tracks".to_string()
    } else {
        filters.join(", ")
    }
}

fn to_percent(count: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        ((count as f64 / total as f64) * 1000.0).round() / 10.0
    }
}

#[derive(Debug, Clone)]
struct TrackProfile {
    track: crate::types::Track,
    camelot_key: Option<CamelotKey>,
    key_display: String,
    bpm: f64,
    energy: f64,
    brightness: Option<f64>,
    rhythm_regularity: Option<f64>,
    loudness_range: Option<f64>,
    canonical_genre: Option<String>,
    genre_family: GenreFamily,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CamelotKey {
    number: u8,
    letter: char,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GenreFamily {
    House,
    Techno,
    Bass,
    Downtempo,
    Other,
}

#[derive(Debug, Clone)]
struct AxisScore {
    value: f64,
    label: String,
}

#[derive(Debug, Clone)]
struct TransitionScores {
    key: AxisScore,
    bpm: AxisScore,
    energy: AxisScore,
    genre: AxisScore,
    brightness: AxisScore,
    rhythm: AxisScore,
    composite: f64,
}

impl TransitionScores {
    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "key": { "value": round_score(self.key.value), "label": self.key.label },
            "bpm": { "value": round_score(self.bpm.value), "label": self.bpm.label },
            "energy": { "value": round_score(self.energy.value), "label": self.energy.label },
            "genre": { "value": round_score(self.genre.value), "label": self.genre.label },
            "brightness": { "value": round_score(self.brightness.value), "label": self.brightness.label },
            "rhythm": { "value": round_score(self.rhythm.value), "label": self.rhythm.label },
            "composite": round_score(self.composite),
        })
    }
}

#[derive(Debug, Clone)]
struct CandidateTransition {
    from_index: usize,
    to_index: usize,
    scores: TransitionScores,
}

#[derive(Debug, Clone)]
struct CandidatePlan {
    ordered_ids: Vec<String>,
    transitions: Vec<CandidateTransition>,
}

fn resolve_energy_curve(
    energy_curve: Option<&EnergyCurveInput>,
    target_tracks: usize,
) -> Result<Vec<EnergyPhase>, String> {
    if target_tracks == 0 {
        return Err("target_tracks must be at least 1".to_string());
    }

    match energy_curve {
        Some(EnergyCurveInput::Custom(phases)) => {
            if phases.len() != target_tracks {
                return Err(format!(
                    "custom phase array length ({}) must match target_tracks ({target_tracks})",
                    phases.len()
                ));
            }
            Ok(phases.clone())
        }
        Some(EnergyCurveInput::Preset(preset)) => Ok((0..target_tracks)
            .map(|position| preset_energy_phase(*preset, position, target_tracks))
            .collect()),
        None => Ok((0..target_tracks)
            .map(|position| {
                preset_energy_phase(
                    EnergyCurvePreset::WarmupBuildPeakRelease,
                    position,
                    target_tracks,
                )
            })
            .collect()),
    }
}

fn preset_energy_phase(preset: EnergyCurvePreset, position: usize, total: usize) -> EnergyPhase {
    let fraction = if total == 0 {
        0.0
    } else {
        position as f64 / total as f64
    };
    match preset {
        EnergyCurvePreset::WarmupBuildPeakRelease => {
            if fraction < 0.15 {
                EnergyPhase::Warmup
            } else if fraction < 0.45 {
                EnergyPhase::Build
            } else if fraction < 0.75 {
                EnergyPhase::Peak
            } else {
                EnergyPhase::Release
            }
        }
        EnergyCurvePreset::Flat => EnergyPhase::Peak,
        EnergyCurvePreset::PeakOnly => {
            if fraction < 0.10 {
                EnergyPhase::Build
            } else if fraction < 0.85 {
                EnergyPhase::Peak
            } else {
                EnergyPhase::Release
            }
        }
    }
}

fn select_start_track_ids(
    profiles_by_id: &HashMap<String, TrackProfile>,
    requested_candidates: usize,
    first_phase: EnergyPhase,
    forced_start: Option<&str>,
) -> Vec<String> {
    if let Some(track_id) = forced_start {
        return vec![track_id.to_string()];
    }

    let prefer_low_energy = matches!(first_phase, EnergyPhase::Warmup | EnergyPhase::Build);
    let mut profiles: Vec<&TrackProfile> = profiles_by_id.values().collect();
    profiles.sort_by(|left, right| {
        let energy_cmp = if prefer_low_energy {
            left.energy
                .partial_cmp(&right.energy)
                .unwrap_or(std::cmp::Ordering::Equal)
        } else {
            right
                .energy
                .partial_cmp(&left.energy)
                .unwrap_or(std::cmp::Ordering::Equal)
        };
        energy_cmp.then_with(|| left.track.id.cmp(&right.track.id))
    });

    let wanted = requested_candidates.max(1);
    let mut out: Vec<String> = profiles
        .into_iter()
        .take(wanted)
        .map(|profile| profile.track.id.clone())
        .collect();
    if out.is_empty() {
        out.extend(profiles_by_id.keys().take(1).cloned());
    }
    out
}

fn build_candidate_plan(
    profiles_by_id: &HashMap<String, TrackProfile>,
    start_track_id: &str,
    target_tracks: usize,
    phases: &[EnergyPhase],
    priority: SetPriority,
    variation_index: usize,
) -> CandidatePlan {
    let mut ordered_ids = vec![start_track_id.to_string()];
    let mut transitions = Vec::new();
    let mut remaining: HashSet<String> = profiles_by_id.keys().cloned().collect();
    remaining.remove(start_track_id);

    while ordered_ids.len() < target_tracks && !remaining.is_empty() {
        let Some(from_track_id) = ordered_ids.last() else {
            break;
        };
        let Some(from_profile) = profiles_by_id.get(from_track_id) else {
            break;
        };

        let to_phase = phases.get(ordered_ids.len()).copied();
        let from_phase = ordered_ids
            .len()
            .checked_sub(1)
            .and_then(|idx| phases.get(idx).copied());
        let mut scored_next: Vec<(String, TransitionScores)> = remaining
            .iter()
            .filter_map(|candidate_id| {
                profiles_by_id.get(candidate_id).map(|to_profile| {
                    (
                        candidate_id.clone(),
                        score_transition_profiles(
                            from_profile,
                            to_profile,
                            from_phase,
                            to_phase,
                            priority,
                        ),
                    )
                })
            })
            .collect();

        scored_next.sort_by(|left, right| {
            right
                .1
                .composite
                .partial_cmp(&left.1.composite)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.0.cmp(&right.0))
        });

        if scored_next.is_empty() {
            break;
        }

        let pick_rank = transition_pick_rank(variation_index, ordered_ids.len(), scored_next.len());
        let (next_track_id, transition_scores) = scored_next[pick_rank].clone();

        transitions.push(CandidateTransition {
            from_index: ordered_ids.len() - 1,
            to_index: ordered_ids.len(),
            scores: transition_scores,
        });
        ordered_ids.push(next_track_id.clone());
        remaining.remove(&next_track_id);
    }

    CandidatePlan {
        ordered_ids,
        transitions,
    }
}

fn transition_pick_rank(
    variation_index: usize,
    current_length: usize,
    available_options: usize,
) -> usize {
    if available_options <= 1 {
        return 0;
    }
    let preferred_rank = if current_length == 1 {
        variation_index
    } else if variation_index > 0 && current_length.is_multiple_of(4) {
        variation_index.min(1)
    } else {
        0
    };
    preferred_rank.min(available_options - 1)
}

fn build_track_profile(
    track: crate::types::Track,
    store_conn: &Connection,
) -> Result<TrackProfile, String> {
    let cache_key = resolve_file_path(&track.file_path).unwrap_or_else(|_| track.file_path.clone());
    let stratum_json = store::get_audio_analysis(store_conn, &cache_key, "stratum-dsp")
        .map_err(|e| format!("stratum cache read error: {e}"))?
        .and_then(|cached| serde_json::from_str::<serde_json::Value>(&cached.features_json).ok());
    let essentia_json = store::get_audio_analysis(store_conn, &cache_key, "essentia")
        .map_err(|e| format!("essentia cache read error: {e}"))?
        .and_then(|cached| serde_json::from_str::<serde_json::Value>(&cached.features_json).ok());

    let bpm = stratum_json
        .as_ref()
        .and_then(|v| v.get("bpm"))
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(track.bpm)
        .max(0.0);

    let camelot_key = stratum_json
        .as_ref()
        .and_then(|v| v.get("key_camelot").and_then(serde_json::Value::as_str))
        .and_then(parse_camelot_key)
        .or_else(|| key_to_camelot(&track.key));

    let key_display = camelot_key
        .map(format_camelot)
        .unwrap_or_else(|| match track.key.trim() {
            "" => "Unknown".to_string(),
            _ => track.key.clone(),
        });

    let energy = compute_track_energy(essentia_json.as_ref(), bpm);
    let brightness = essentia_json
        .as_ref()
        .and_then(|v| v.get("spectral_centroid_mean"))
        .and_then(serde_json::Value::as_f64);
    let rhythm_regularity = essentia_json
        .as_ref()
        .and_then(|v| v.get("rhythm_regularity"))
        .and_then(serde_json::Value::as_f64);
    let loudness_range = essentia_json
        .as_ref()
        .and_then(|v| v.get("loudness_range"))
        .and_then(serde_json::Value::as_f64);
    let canonical_genre = canonicalize_genre(&track.genre);
    let genre_family = canonical_genre
        .as_deref()
        .map(genre_family_for)
        .unwrap_or(GenreFamily::Other);

    Ok(TrackProfile {
        track,
        camelot_key,
        key_display,
        bpm,
        energy,
        brightness,
        rhythm_regularity,
        loudness_range,
        canonical_genre,
        genre_family,
    })
}

fn score_transition_profiles(
    from: &TrackProfile,
    to: &TrackProfile,
    from_phase: Option<EnergyPhase>,
    to_phase: Option<EnergyPhase>,
    priority: SetPriority,
) -> TransitionScores {
    let key = score_key_axis(from.camelot_key, to.camelot_key);
    let bpm = score_bpm_axis(from.bpm, to.bpm);
    let energy = score_energy_axis(
        from.energy,
        to.energy,
        from_phase,
        to_phase,
        to.loudness_range,
    );
    let genre = score_genre_axis(
        from.canonical_genre.as_deref(),
        to.canonical_genre.as_deref(),
        from.genre_family,
        to.genre_family,
    );
    let brightness = score_brightness_axis(from.brightness, to.brightness);
    let rhythm = score_rhythm_axis(from.rhythm_regularity, to.rhythm_regularity);
    let brightness_available = from.brightness.is_some() && to.brightness.is_some();
    let rhythm_available = from.rhythm_regularity.is_some() && to.rhythm_regularity.is_some();
    let composite = composite_score(
        key.value,
        bpm.value,
        energy.value,
        genre.value,
        if brightness_available {
            Some(brightness.value)
        } else {
            None
        },
        if rhythm_available {
            Some(rhythm.value)
        } else {
            None
        },
        priority,
    );

    TransitionScores {
        key,
        bpm,
        energy,
        genre,
        brightness,
        rhythm,
        composite,
    }
}

fn round_score(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

fn score_key_axis(from: Option<CamelotKey>, to: Option<CamelotKey>) -> AxisScore {
    let Some(from) = from else {
        return AxisScore {
            value: 0.1,
            label: "Clash (missing key)".to_string(),
        };
    };
    let Some(to) = to else {
        return AxisScore {
            value: 0.1,
            label: "Clash (missing key)".to_string(),
        };
    };

    if from.number == to.number && from.letter == to.letter {
        return AxisScore {
            value: 1.0,
            label: "Perfect".to_string(),
        };
    }
    if from.number == to.number && from.letter != to.letter {
        return AxisScore {
            value: 0.8,
            label: "Mood shift (A↔B)".to_string(),
        };
    }

    let clockwise = ((to.number as i16 - from.number as i16 + 12) % 12) as u8;
    if from.letter == to.letter && clockwise == 1 {
        AxisScore {
            value: 0.9,
            label: "Energy boost (+1)".to_string(),
        }
    } else if from.letter == to.letter && clockwise == 11 {
        AxisScore {
            value: 0.9,
            label: "Energy drop (-1)".to_string(),
        }
    } else if from.letter == to.letter && (clockwise == 2 || clockwise == 10) {
        AxisScore {
            value: 0.5,
            label: "Acceptable (+/-2)".to_string(),
        }
    } else if from.letter != to.letter && (clockwise == 1 || clockwise == 11) {
        AxisScore {
            value: 0.4,
            label: "Rough (+/-1, A↔B)".to_string(),
        }
    } else {
        AxisScore {
            value: 0.1,
            label: "Clash".to_string(),
        }
    }
}

fn score_bpm_axis(from_bpm: f64, to_bpm: f64) -> AxisScore {
    let delta = (from_bpm - to_bpm).abs();
    if delta <= 2.0 {
        AxisScore {
            value: 1.0,
            label: format!("Seamless (delta {:.1})", delta),
        }
    } else if delta <= 4.0 {
        AxisScore {
            value: 0.8,
            label: format!("Comfortable pitch adjust (delta {:.1})", delta),
        }
    } else if delta <= 6.0 {
        AxisScore {
            value: 0.5,
            label: format!("Noticeable (delta {:.1})", delta),
        }
    } else if delta <= 8.0 {
        AxisScore {
            value: 0.3,
            label: format!("Needs creative transition (delta {:.1})", delta),
        }
    } else {
        AxisScore {
            value: 0.1,
            label: format!("Likely jarring (delta {:.1})", delta),
        }
    }
}

fn score_energy_axis(
    from_energy: f64,
    to_energy: f64,
    from_phase: Option<EnergyPhase>,
    to_phase: Option<EnergyPhase>,
    to_loudness_range: Option<f64>,
) -> AxisScore {
    let delta = to_energy - from_energy;
    let mut axis = match to_phase {
        Some(EnergyPhase::Warmup) => {
            let met = (-0.03..=0.12).contains(&delta);
            AxisScore {
                value: if met { 1.0 } else { 0.5 },
                label: if met {
                    "Stable/slight rise (warmup phase)".to_string()
                } else {
                    "Too abrupt for warmup".to_string()
                },
            }
        }
        Some(EnergyPhase::Build) => {
            let met = delta >= 0.03;
            AxisScore {
                value: if met { 1.0 } else { 0.3 },
                label: if met {
                    "Rising (build phase)".to_string()
                } else {
                    "Not rising (build phase)".to_string()
                },
            }
        }
        Some(EnergyPhase::Peak) => {
            let met = to_energy >= 0.65 && delta.abs() <= 0.10;
            AxisScore {
                value: if met { 1.0 } else { 0.5 },
                label: if met {
                    "High and stable (peak phase)".to_string()
                } else {
                    "Not high/stable (peak phase)".to_string()
                },
            }
        }
        Some(EnergyPhase::Release) => {
            let met = delta <= -0.03;
            AxisScore {
                value: if met { 1.0 } else { 0.3 },
                label: if met {
                    "Dropping (release phase)".to_string()
                } else {
                    "Not dropping (release phase)".to_string()
                },
            }
        }
        None => AxisScore {
            value: 1.0,
            label: "No phase preference".to_string(),
        },
    };

    let is_phase_boundary = matches!(
        (from_phase, to_phase),
        (Some(previous), Some(current)) if previous != current
    );
    match (to_phase, to_loudness_range) {
        (Some(_), Some(lra)) if is_phase_boundary && lra > 8.0 => {
            axis.value = (axis.value + 0.1).clamp(0.0, 1.0);
            axis.label.push_str(" + dynamic boundary boost");
        }
        (Some(EnergyPhase::Peak), Some(lra)) if !is_phase_boundary && lra < 4.0 => {
            axis.value = (axis.value + 0.05).clamp(0.0, 1.0);
            axis.label.push_str(" + sustained-peak consistency boost");
        }
        _ => {}
    }
    axis
}

fn score_genre_axis(
    from_genre: Option<&str>,
    to_genre: Option<&str>,
    from_family: GenreFamily,
    to_family: GenreFamily,
) -> AxisScore {
    let Some(from_genre) = from_genre else {
        return AxisScore {
            value: 0.5,
            label: "Unknown genre".to_string(),
        };
    };
    let Some(to_genre) = to_genre else {
        return AxisScore {
            value: 0.5,
            label: "Unknown genre".to_string(),
        };
    };

    if from_genre.eq_ignore_ascii_case(to_genre) {
        AxisScore {
            value: 1.0,
            label: "Same genre".to_string(),
        }
    } else if from_family == to_family && from_family != GenreFamily::Other {
        AxisScore {
            value: 0.7,
            label: "Same family".to_string(),
        }
    } else {
        AxisScore {
            value: 0.3,
            label: "Different families".to_string(),
        }
    }
}

fn score_brightness_axis(from_centroid: Option<f64>, to_centroid: Option<f64>) -> AxisScore {
    let Some(from_centroid) = from_centroid else {
        return AxisScore {
            value: 0.5,
            label: "Unknown brightness".to_string(),
        };
    };
    let Some(to_centroid) = to_centroid else {
        return AxisScore {
            value: 0.5,
            label: "Unknown brightness".to_string(),
        };
    };

    let delta = (to_centroid - from_centroid).abs();
    if delta < 300.0 {
        AxisScore {
            value: 1.0,
            label: format!("Similar timbre (delta {:.0} Hz)", delta),
        }
    } else if delta < 800.0 {
        AxisScore {
            value: 0.7,
            label: format!("Noticeable brightness shift (delta {:.0} Hz)", delta),
        }
    } else if delta < 1500.0 {
        AxisScore {
            value: 0.4,
            label: format!("Large timbral jump (delta {:.0} Hz)", delta),
        }
    } else {
        AxisScore {
            value: 0.2,
            label: format!("Jarring brightness jump (delta {:.0} Hz)", delta),
        }
    }
}

fn score_rhythm_axis(from_regularity: Option<f64>, to_regularity: Option<f64>) -> AxisScore {
    let Some(from_regularity) = from_regularity else {
        return AxisScore {
            value: 0.5,
            label: "Unknown groove".to_string(),
        };
    };
    let Some(to_regularity) = to_regularity else {
        return AxisScore {
            value: 0.5,
            label: "Unknown groove".to_string(),
        };
    };

    let delta = (to_regularity - from_regularity).abs();
    if delta < 0.1 {
        AxisScore {
            value: 1.0,
            label: format!("Matching groove (delta {:.2})", delta),
        }
    } else if delta < 0.25 {
        AxisScore {
            value: 0.7,
            label: format!("Manageable groove shift (delta {:.2})", delta),
        }
    } else if delta < 0.5 {
        AxisScore {
            value: 0.4,
            label: format!("Challenging groove shift (delta {:.2})", delta),
        }
    } else {
        AxisScore {
            value: 0.2,
            label: format!("Groove clash (delta {:.2})", delta),
        }
    }
}

fn priority_weights(priority: SetPriority) -> (f64, f64, f64, f64, f64, f64) {
    match priority {
        SetPriority::Balanced => (0.30, 0.20, 0.18, 0.17, 0.08, 0.07),
        SetPriority::Harmonic => (0.48, 0.18, 0.12, 0.08, 0.08, 0.06),
        SetPriority::Energy => (0.12, 0.18, 0.42, 0.12, 0.08, 0.08),
        SetPriority::Genre => (0.18, 0.18, 0.12, 0.38, 0.08, 0.06),
    }
}

fn composite_score(
    key_score: f64,
    bpm_score: f64,
    energy_score: f64,
    genre_score: f64,
    brightness_score: Option<f64>,
    rhythm_score: Option<f64>,
    priority: SetPriority,
) -> f64 {
    let (w_key, w_bpm, w_energy, w_genre, w_brightness, w_rhythm) = priority_weights(priority);
    let mut weighted_sum = (w_key * key_score)
        + (w_bpm * bpm_score)
        + (w_energy * energy_score)
        + (w_genre * genre_score);
    let mut total_weight = w_key + w_bpm + w_energy + w_genre;

    if let Some(brightness) = brightness_score {
        weighted_sum += w_brightness * brightness;
        total_weight += w_brightness;
    }
    if let Some(rhythm) = rhythm_score {
        weighted_sum += w_rhythm * rhythm;
        total_weight += w_rhythm;
    }

    if total_weight <= f64::EPSILON {
        0.0
    } else {
        weighted_sum / total_weight
    }
}

fn compute_track_energy(essentia_json: Option<&serde_json::Value>, bpm: f64) -> f64 {
    // Fallback proxy when Essentia descriptors are unavailable.
    // This maps typical club tempos (~95-145 BPM) across the full 0-1 range.
    let bpm_proxy = ((bpm - 95.0) / 50.0).clamp(0.0, 1.0);
    let Some(essentia_json) = essentia_json else {
        return bpm_proxy;
    };

    let danceability = essentia_json
        .get("danceability")
        .and_then(serde_json::Value::as_f64);
    let loudness_integrated = essentia_json
        .get("loudness_integrated")
        .and_then(serde_json::Value::as_f64);
    let onset_rate = essentia_json
        .get("onset_rate")
        .and_then(serde_json::Value::as_f64);

    match (danceability, loudness_integrated, onset_rate) {
        (Some(dance), Some(loudness), Some(onset)) => {
            let normalized_dance = (dance / 3.0).clamp(0.0, 1.0);
            let normalized_loudness = ((loudness + 30.0) / 30.0).clamp(0.0, 1.0);
            let onset_rate_normalized = (onset / 10.0).clamp(0.0, 1.0);
            ((0.4 * normalized_dance) + (0.3 * normalized_loudness) + (0.3 * onset_rate_normalized))
                .clamp(0.0, 1.0)
        }
        _ => bpm_proxy,
    }
}

fn canonicalize_genre(raw_genre: &str) -> Option<String> {
    let trimmed = raw_genre.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(canonical) = genre::canonical_casing(trimmed) {
        return Some(canonical.to_string());
    }
    if let Some(alias_target) = genre::normalize_genre(trimmed) {
        return Some(alias_target.to_string());
    }
    None
}

fn genre_family_for(canonical_genre: &str) -> GenreFamily {
    match canonical_genre.trim().to_ascii_lowercase().as_str() {
        "house" | "deep house" | "tech house" | "afro house" | "garage" | "speed garage" => {
            GenreFamily::House
        }
        "techno" | "deep techno" | "minimal" | "dub techno" | "ambient techno" | "hard techno"
        | "acid" | "electro" => GenreFamily::Techno,
        "drum & bass" | "drum and bass" | "jungle" | "dubstep" | "breakbeat" | "uk bass"
        | "grime" | "bassline" | "broken beat" => GenreFamily::Bass,
        "ambient" | "downtempo" | "dub" | "idm" | "experimental" => GenreFamily::Downtempo,
        "hip hop" | "disco" | "trance" | "psytrance" | "pop" | "r&b" | "rnb" | "reggae"
        | "dancehall" | "rock" | "synth-pop" | "synth pop" => GenreFamily::Other,
        _ => GenreFamily::Other,
    }
}

fn key_to_camelot(raw_key: &str) -> Option<CamelotKey> {
    parse_camelot_key(raw_key).or_else(|| standard_key_to_camelot(raw_key))
}

fn parse_camelot_key(raw_key: &str) -> Option<CamelotKey> {
    let trimmed = raw_key.trim().to_ascii_uppercase();
    if trimmed.len() < 2 {
        return None;
    }
    let (number, letter_str) = trimmed.split_at(trimmed.len() - 1);
    let letter = letter_str.chars().next()?;
    if letter != 'A' && letter != 'B' {
        return None;
    }
    let number: u8 = number.parse().ok()?;
    if !(1..=12).contains(&number) {
        return None;
    }
    Some(CamelotKey { number, letter })
}

fn standard_key_to_camelot(raw_key: &str) -> Option<CamelotKey> {
    let normalized = raw_key.trim().replace('♯', "#").replace('♭', "b");
    if normalized.is_empty() {
        return None;
    }
    let lower = normalized.to_ascii_lowercase();

    let (root_raw, is_minor) = if lower.ends_with("minor") && normalized.len() > 5 {
        (&normalized[..normalized.len() - 5], true)
    } else if lower.ends_with("min") && normalized.len() > 3 {
        (&normalized[..normalized.len() - 3], true)
    } else if lower.ends_with('m') && normalized.len() > 1 {
        (&normalized[..normalized.len() - 1], true)
    } else if lower.ends_with("major") && normalized.len() > 5 {
        (&normalized[..normalized.len() - 5], false)
    } else if lower.ends_with("maj") && normalized.len() > 3 {
        (&normalized[..normalized.len() - 3], false)
    } else {
        (normalized.as_str(), false)
    };
    let root = normalize_key_root(root_raw)?;

    let (number, letter) = if is_minor {
        match root.as_str() {
            "G#" | "Ab" => (1, 'A'),
            "D#" | "Eb" => (2, 'A'),
            "A#" | "Bb" => (3, 'A'),
            "F" => (4, 'A'),
            "C" => (5, 'A'),
            "G" => (6, 'A'),
            "D" => (7, 'A'),
            "A" => (8, 'A'),
            "E" => (9, 'A'),
            "B" => (10, 'A'),
            "F#" | "Gb" => (11, 'A'),
            "C#" | "Db" => (12, 'A'),
            _ => return None,
        }
    } else {
        match root.as_str() {
            "B" => (1, 'B'),
            "F#" | "Gb" => (2, 'B'),
            "C#" | "Db" => (3, 'B'),
            "G#" | "Ab" => (4, 'B'),
            "D#" | "Eb" => (5, 'B'),
            "A#" | "Bb" => (6, 'B'),
            "F" => (7, 'B'),
            "C" => (8, 'B'),
            "G" => (9, 'B'),
            "D" => (10, 'B'),
            "A" => (11, 'B'),
            "E" => (12, 'B'),
            _ => return None,
        }
    };
    Some(CamelotKey { number, letter })
}

fn normalize_key_root(root: &str) -> Option<String> {
    let stripped: String = root.chars().filter(|ch| !ch.is_whitespace()).collect();
    if stripped.is_empty() {
        return None;
    }
    let mut chars = stripped.chars();
    let letter = chars.next()?.to_ascii_uppercase();
    if !matches!(letter, 'A' | 'B' | 'C' | 'D' | 'E' | 'F' | 'G') {
        return None;
    }

    let accidental = chars.next();
    if chars.next().is_some() {
        return None;
    }

    let normalized = match accidental {
        Some('#') => format!("{letter}#"),
        Some('b') | Some('B') => format!("{letter}b"),
        Some(_) => return None,
        None => letter.to_string(),
    };
    Some(normalized)
}

fn format_camelot(key: CamelotKey) -> String {
    format!("{}{}", key.number, key.letter)
}

/// Map a genre/style string through the taxonomy.
/// Returns (maps_to, mapping_type) where mapping_type is "exact", "alias", or "unknown".
fn map_genre_through_taxonomy(style: &str) -> (Option<String>, &'static str) {
    if let Some(canonical) = genre::canonical_casing(style) {
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

fn resolve_file_path(raw_path: &str) -> Result<String, McpError> {
    audio::resolve_audio_path(raw_path).map_err(err)
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
            ReklawdboxServer::new(None).serve(server_io),
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
            .user_agent("Reklawdbox/0.1")
            .build()
            .expect("default test HTTP client should build")
    }

    fn create_server_with_connections(
        db_conn: Connection,
        store_conn: Connection,
        http: reqwest::Client,
    ) -> ReklawdboxServer {
        let server = ReklawdboxServer {
            state: Arc::new(ServerState {
                db: OnceLock::new(),
                internal_db: OnceLock::new(),
                essentia_python: OnceLock::new(),
                essentia_python_override: Mutex::new(None),
                essentia_setup_lock: tokio::sync::Mutex::new(()),
                discogs_pending: Mutex::new(None),
                db_path: None,
                changes: ChangeManager::new(),
                http,
            }),
            tool_router: ReklawdboxServer::tool_router(),
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
    ) -> Option<(ReklawdboxServer, TempDir)> {
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

    fn sample_real_tracks(server: &ReklawdboxServer, limit: u32) -> Vec<crate::types::Track> {
        let conn = server
            .conn()
            .expect("real DB connection should be available for integration test");
        db::search_tracks(
            &conn,
            &db::SearchParams {
                has_genre: Some(true),
                exclude_samples: true,
                limit: Some(limit),
                ..Default::default()
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

    fn insert_test_track(
        conn: &Connection,
        track_id: &str,
        title: &str,
        genre_id: &str,
        file_path: &str,
    ) {
        conn.execute(
            "INSERT INTO djmdContent (
                ID, Title, ArtistID, AlbumID, GenreID, KeyID, ColorID, LabelID, RemixerID,
                BPM, Rating, Commnt, ReleaseYear, Length, FolderPath, DJPlayCount, BitRate,
                SampleRate, FileType, created_at, rb_local_deleted
            ) VALUES (
                ?1, ?2, 'a1', 'al1', ?3, 'k1', 'c1', 'l1', '',
                12700, 102, 'cache coverage test', 2025, 220, ?4, '0', 1411,
                44100, 5, '2025-01-02', 0
            )",
            params![track_id, title, genre_id, file_path],
        )
        .expect("test track should insert");
    }

    fn canonical_genre_name(raw_genre: &str) -> String {
        if let Some(canonical) = genre::canonical_casing(raw_genre) {
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

    fn create_build_set_test_db() -> (Connection, Vec<String>) {
        let conn = create_single_track_test_db("set-track-1", "/tmp/set-track-1.flac");
        conn.execute_batch(
            "
            INSERT INTO djmdGenre (ID, Name) VALUES ('g2', 'House');
            INSERT INTO djmdGenre (ID, Name) VALUES ('g3', 'Tech House');

            INSERT INTO djmdKey (ID, ScaleName) VALUES ('k2', 'Em');
            INSERT INTO djmdKey (ID, ScaleName) VALUES ('k3', 'Bm');
            INSERT INTO djmdKey (ID, ScaleName) VALUES ('k4', 'F#m');
            INSERT INTO djmdKey (ID, ScaleName) VALUES ('k5', 'C#m');
            INSERT INTO djmdKey (ID, ScaleName) VALUES ('k6', 'Dm');
            ",
        )
        .expect("build_set fixture taxonomy inserts should succeed");

        let tracks: [(&str, &str, &str, &str, i32, i32); 5] = [
            ("set-track-2", "Second Step", "g1", "k2", 12400, 300),
            ("set-track-3", "Third Wave", "g2", "k3", 12600, 0),
            ("set-track-4", "Fourth Lift", "g3", "k4", 12800, 360),
            ("set-track-5", "Fifth Peak", "g3", "k5", 12950, 420),
            ("set-track-6", "Sixth Release", "g2", "k6", 12350, 250),
        ];

        for (index, (track_id, title, genre_id, key_id, bpm, length)) in tracks.iter().enumerate() {
            conn.execute(
                "INSERT INTO djmdContent (
                    ID, Title, ArtistID, AlbumID, GenreID, KeyID, ColorID, LabelID, RemixerID,
                    BPM, Rating, Commnt, ReleaseYear, Length, FolderPath, DJPlayCount, BitRate,
                    SampleRate, FileType, created_at, rb_local_deleted
                ) VALUES (
                    ?1, ?2, 'a1', 'al1', ?3, ?4, 'c1', 'l1', '',
                    ?5, 153, 'build_set fixture', 2025, ?6, ?7, '0', 1411,
                    44100, 5, '2025-01-03', 0
                )",
                params![
                    *track_id,
                    *title,
                    *genre_id,
                    *key_id,
                    *bpm,
                    *length,
                    format!("/tmp/{track_id}.flac"),
                ],
            )
            .unwrap_or_else(|e| panic!("fixture track insert {index} should succeed: {e}"));
        }

        (
            conn,
            vec![
                "set-track-1".to_string(),
                "set-track-2".to_string(),
                "set-track-3".to_string(),
                "set-track-4".to_string(),
                "set-track-5".to_string(),
                "set-track-6".to_string(),
            ],
        )
    }

    fn seed_build_set_cache(store_conn: &Connection) {
        let rows: [(&str, f64, &str, f64); 6] = [
            ("/tmp/set-track-1.flac", 122.0, "8A", 1.02),
            ("/tmp/set-track-2.flac", 124.0, "9A", 1.20),
            ("/tmp/set-track-3.flac", 126.0, "10A", 1.44),
            ("/tmp/set-track-4.flac", 128.0, "11A", 1.80),
            ("/tmp/set-track-5.flac", 130.0, "12A", 2.22),
            ("/tmp/set-track-6.flac", 123.5, "7A", 1.26),
        ];

        for (index, (path, bpm, key_camelot, danceability)) in rows.iter().enumerate() {
            let stratum = serde_json::json!({
                "bpm": *bpm,
                "key": "Am",
                "key_camelot": *key_camelot,
                "analyzer_version": "stratum-dsp-test"
            });
            let essentia = serde_json::json!({
                "danceability": *danceability,
                "loudness_integrated": -18.0 + (*danceability * 4.0),
                "onset_rate": 2.5 + (*danceability * 2.0),
                "analyzer_version": "essentia-test"
            });
            store::set_audio_analysis(
                store_conn,
                path,
                "stratum-dsp",
                1000 + index as i64,
                2000 + index as i64,
                "stratum-dsp-test",
                &stratum.to_string(),
            )
            .unwrap_or_else(|e| panic!("stratum cache seed {index} should succeed: {e}"));
            store::set_audio_analysis(
                store_conn,
                path,
                "essentia",
                1000 + index as i64,
                2000 + index as i64,
                "essentia-test",
                &essentia.to_string(),
            )
            .unwrap_or_else(|e| panic!("essentia cache seed {index} should succeed: {e}"));
        }
    }

    #[tokio::test]
    async fn build_set_generates_candidates_and_transition_scores() {
        let (db_conn, track_ids) = create_build_set_test_db();
        let store_dir = tempfile::tempdir().expect("temp store dir should create");
        let store_path = store_dir.path().join("internal.sqlite3");
        let store_conn = store::open(
            store_path
                .to_str()
                .expect("temp store path should be UTF-8"),
        )
        .expect("temp internal store should open");
        seed_build_set_cache(&store_conn);

        let server =
            create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());
        let result = server
            .build_set(Parameters(BuildSetParams {
                track_ids,
                target_tracks: 4,
                priority: Some(SetPriority::Balanced),
                energy_curve: Some(EnergyCurveInput::Preset(
                    EnergyCurvePreset::WarmupBuildPeakRelease,
                )),
                start_track_id: None,
                candidates: Some(3),
            }))
            .await
            .expect("build_set should succeed for fixture pool");
        let payload = extract_json(&result);

        assert_eq!(payload["pool_size"], 6);
        assert_eq!(payload["tracks_used"], 4);
        let candidates = payload["candidates"]
            .as_array()
            .expect("candidates should be an array");
        assert_eq!(candidates.len(), 3);

        for candidate in candidates {
            let tracks = candidate["tracks"]
                .as_array()
                .expect("candidate tracks should be an array");
            let transitions = candidate["transitions"]
                .as_array()
                .expect("candidate transitions should be an array");
            assert_eq!(tracks.len(), 4);
            assert_eq!(transitions.len(), 3);
            assert!(
                candidate["set_score"].as_f64().is_some(),
                "set_score should be numeric"
            );
            let set_score = candidate["set_score"]
                .as_f64()
                .expect("set_score should be numeric");
            assert!(
                (set_score - round_score(set_score)).abs() < 1e-9,
                "set_score should be rounded to 3 decimal places"
            );
            assert!(
                candidate["estimated_duration_minutes"].as_i64().is_some(),
                "estimated_duration_minutes should be numeric"
            );
            for transition in transitions {
                assert!(
                    transition["scores"]["composite"].as_f64().is_some(),
                    "each transition should include numeric composite score"
                );
            }
        }

        let candidate_a_ids: Vec<String> = candidates[0]["tracks"]
            .as_array()
            .expect("candidate A tracks array")
            .iter()
            .map(|track| {
                track["track_id"]
                    .as_str()
                    .expect("candidate track should include track_id")
                    .to_string()
            })
            .collect();
        let candidate_b_ids: Vec<String> = candidates[1]["tracks"]
            .as_array()
            .expect("candidate B tracks array")
            .iter()
            .map(|track| {
                track["track_id"]
                    .as_str()
                    .expect("candidate track should include track_id")
                    .to_string()
            })
            .collect();
        assert_ne!(
            candidate_a_ids, candidate_b_ids,
            "candidate generation should include variation"
        );
    }

    #[tokio::test]
    async fn build_set_custom_curve_and_small_pool_are_handled() {
        let db_conn = create_single_track_test_db("single-set-1", "/tmp/single-set-1.flac");
        db_conn
            .execute(
                "UPDATE djmdContent SET Length = 0 WHERE ID = ?1",
                params!["single-set-1"],
            )
            .expect("single-track fixture should update");

        let store_dir = tempfile::tempdir().expect("temp store dir should create");
        let store_path = store_dir.path().join("internal.sqlite3");
        let store_conn = store::open(
            store_path
                .to_str()
                .expect("temp store path should be UTF-8"),
        )
        .expect("temp internal store should open");

        let server =
            create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());
        let result = server
            .build_set(Parameters(BuildSetParams {
                track_ids: vec!["single-set-1".to_string()],
                target_tracks: 4,
                priority: Some(SetPriority::Energy),
                energy_curve: Some(EnergyCurveInput::Custom(vec![
                    EnergyPhase::Warmup,
                    EnergyPhase::Build,
                    EnergyPhase::Peak,
                    EnergyPhase::Release,
                ])),
                start_track_id: None,
                candidates: Some(2),
            }))
            .await
            .expect("build_set should succeed for single-track pool");
        let payload = extract_json(&result);

        assert_eq!(payload["pool_size"], 1);
        assert_eq!(payload["tracks_used"], 1);
        let candidates = payload["candidates"]
            .as_array()
            .expect("candidates should be an array");
        assert_eq!(candidates.len(), 1);
        let first = &candidates[0];
        assert_eq!(
            first["tracks"]
                .as_array()
                .expect("tracks should be array")
                .len(),
            1
        );
        assert_eq!(
            first["transitions"]
                .as_array()
                .expect("transitions should be array")
                .len(),
            0
        );
        assert_eq!(
            first["estimated_duration_minutes"]
                .as_i64()
                .expect("duration should be integer"),
            6
        );
    }

    #[tokio::test]
    async fn build_set_handles_all_same_key_pool() {
        let db_conn = create_single_track_test_db("same-key-1", "/tmp/same-key-1.flac");
        insert_test_track(
            &db_conn,
            "same-key-2",
            "Same Key Two",
            "g1",
            "/tmp/same-key-2.flac",
        );
        insert_test_track(
            &db_conn,
            "same-key-3",
            "Same Key Three",
            "g1",
            "/tmp/same-key-3.flac",
        );

        let store_dir = tempfile::tempdir().expect("temp store dir should create");
        let store_path = store_dir.path().join("internal.sqlite3");
        let store_conn = store::open(
            store_path
                .to_str()
                .expect("temp store path should be UTF-8"),
        )
        .expect("temp internal store should open");

        let server =
            create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());
        let result = server
            .build_set(Parameters(BuildSetParams {
                track_ids: vec![
                    "same-key-1".to_string(),
                    "same-key-2".to_string(),
                    "same-key-3".to_string(),
                ],
                target_tracks: 3,
                priority: Some(SetPriority::Harmonic),
                energy_curve: Some(EnergyCurveInput::Preset(EnergyCurvePreset::Flat)),
                start_track_id: None,
                candidates: Some(2),
            }))
            .await
            .expect("build_set should succeed when all tracks share the same key");
        let payload = extract_json(&result);

        assert_eq!(payload["pool_size"], 3);
        assert_eq!(payload["tracks_used"], 3);
        let candidates = payload["candidates"]
            .as_array()
            .expect("candidates should be an array");
        assert_eq!(candidates.len(), 1);
        assert_eq!(
            candidates[0]["transitions"]
                .as_array()
                .expect("transitions should be an array")
                .len(),
            2
        );
    }

    #[tokio::test]
    async fn build_set_recomputes_preset_curve_when_pool_is_smaller_than_target() {
        let (db_conn, track_ids) = create_build_set_test_db();
        let selected: Vec<String> = track_ids.into_iter().take(3).collect();

        let store_dir = tempfile::tempdir().expect("temp store dir should create");
        let store_path = store_dir.path().join("internal.sqlite3");
        let store_conn = store::open(
            store_path
                .to_str()
                .expect("temp store path should be UTF-8"),
        )
        .expect("temp internal store should open");
        seed_build_set_cache(&store_conn);

        let server =
            create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());
        let result = server
            .build_set(Parameters(BuildSetParams {
                track_ids: selected,
                target_tracks: 6,
                priority: Some(SetPriority::Balanced),
                energy_curve: Some(EnergyCurveInput::Preset(
                    EnergyCurvePreset::WarmupBuildPeakRelease,
                )),
                start_track_id: None,
                candidates: Some(1),
            }))
            .await
            .expect("build_set should succeed when pool is smaller than target");
        let payload = extract_json(&result);

        assert_eq!(payload["tracks_used"], 3);
        let transitions = payload["candidates"][0]["transitions"]
            .as_array()
            .expect("candidate transitions should be an array");
        assert_eq!(transitions.len(), 2);
        let second_energy_label = transitions[1]["scores"]["energy"]["label"]
            .as_str()
            .expect("second transition should include energy label");
        assert!(
            second_energy_label.contains("peak phase"),
            "phase scaling should include a peak phase for the final transition when tracks_used=3; got: {second_energy_label}"
        );
    }

    #[test]
    #[cfg(unix)]
    fn probe_essentia_python_prefers_env_override_when_valid() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("temp dir should create");
        let fake_python = dir.path().join("fake-python");
        std::fs::write(&fake_python, "#!/bin/sh\necho '2.1b6.dev1389'\nexit 0\n")
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

    #[test]
    #[cfg(unix)]
    fn probe_essentia_python_rejects_success_without_version_output() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("temp dir should create");
        let fake_python = dir.path().join("fake-python-empty");
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
            Some(dir.path().join("other")),
        );
        assert!(
            resolved.is_none(),
            "probe should reject candidates that do not emit version output"
        );
    }

    #[test]
    #[cfg(unix)]
    fn validate_essentia_python_times_out() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("temp dir should create");
        let fake_python = dir.path().join("fake-python-slow");
        std::fs::write(&fake_python, "#!/bin/sh\nsleep 2\necho '2.1b6.dev1389'\n")
            .expect("fake python script should be written");
        let mut perms = std::fs::metadata(&fake_python)
            .expect("fake python metadata should be readable")
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&fake_python, perms)
            .expect("fake python script should be executable");

        let start = std::time::Instant::now();
        let is_valid = validate_essentia_python_with_timeout(
            fake_python.to_str().unwrap(),
            Duration::from_millis(100),
        );
        assert!(
            !is_valid,
            "slow candidate should be rejected when probe timeout elapses"
        );
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "probe timeout should fail fast"
        );
    }

    #[test]
    fn lookup_output_with_cache_metadata_normalizes_non_object_payloads() {
        let output = lookup_output_with_cache_metadata(serde_json::Value::Null, false, None);
        assert_eq!(output["result"], serde_json::Value::Null);
        assert_eq!(output["cache_hit"], false);
        assert!(
            output.get("cached_at").is_none(),
            "live payload should not include cached_at"
        );
    }

    #[test]
    fn lookup_output_with_cache_metadata_keeps_object_payload_shape() {
        let output = lookup_output_with_cache_metadata(
            serde_json::json!({
                "genre": "Techno"
            }),
            true,
            Some("2026-02-20T10:00:00Z"),
        );
        assert_eq!(output["genre"], "Techno");
        assert_eq!(output["cache_hit"], true);
        assert_eq!(output["cached_at"], "2026-02-20T10:00:00Z");
        assert!(
            output.get("result").is_none(),
            "object payloads should not be wrapped in a result envelope"
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
        let hint = payload["essentia_setup_hint"]
            .as_str()
            .expect("essentia_setup_hint should be present when unavailable");
        assert!(
            hint.contains("setup_essentia"),
            "hint should mention setup_essentia tool"
        );
        assert!(
            hint.contains("CRATE_DIG_ESSENTIA_PYTHON"),
            "hint should mention the env var that was checked"
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

        let track = sample_real_tracks(&server, 40)
            .into_iter()
            .filter(|t| (120.0..=145.0).contains(&t.bpm))
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
        let onset_rate = first_payload["essentia"]["onset_rate"]
            .as_f64()
            .expect("onset_rate should be present in Essentia output");
        let danceability = first_payload["essentia"]["danceability"]
            .as_f64()
            .expect("danceability should be present in Essentia output");
        let loudness_integrated = first_payload["essentia"]["loudness_integrated"]
            .as_f64()
            .expect("loudness_integrated should be present in Essentia output");
        assert!(
            onset_rate > 1.0,
            "onset_rate should be rate-like (Hz), got {onset_rate}"
        );
        assert!(
            (0.0..=3.5).contains(&danceability),
            "danceability should stay in plausible Essentia range [0, ~3], got {danceability}"
        );
        assert!(
            (-30.0..=0.0).contains(&loudness_integrated),
            "loudness_integrated should be in a plausible LUFS range, got {loudness_integrated}"
        );

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
    #[cfg(unix)]
    async fn setup_essentia_returns_already_installed_when_override_is_valid() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("temp dir should create");
        let fake_python = dir.path().join("fake-python");
        std::fs::write(&fake_python, "#!/bin/sh\necho '2.1b6.dev1389'\nexit 0\n")
            .expect("fake python script should be written");
        let mut perms = std::fs::metadata(&fake_python)
            .expect("metadata should be readable")
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&fake_python, perms).expect("fake python should be executable");
        let fake_path = fake_python.to_string_lossy().to_string();

        let server = ReklawdboxServer::new(None);
        {
            let mut guard = server.state.essentia_python_override.lock().unwrap();
            *guard = Some(fake_path.clone());
        }

        let result = server
            .setup_essentia()
            .await
            .expect("setup_essentia should succeed when already installed");
        let payload = extract_json(&result);

        assert_eq!(payload["status"], "already_installed");
        assert_eq!(payload["python_path"], fake_path.as_str());
    }

    #[tokio::test]
    async fn essentia_python_override_takes_precedence() {
        let server = ReklawdboxServer::new(None);
        // Seed the OnceLock probe to None (not found)
        server
            .state
            .essentia_python
            .set(None)
            .expect("essentia probe should be seeded once");
        assert!(
            server.essentia_python_path().is_none(),
            "should be None before override"
        );

        // Set an override
        {
            let mut guard = server.state.essentia_python_override.lock().unwrap();
            *guard = Some("/override/python".to_string());
        }
        assert_eq!(
            server.essentia_python_path().as_deref(),
            Some("/override/python"),
            "override should take precedence over OnceLock probe"
        );
    }

    #[tokio::test]
    async fn write_xml_no_change_path_includes_provenance() {
        let server = ReklawdboxServer::new(None);

        let result = server
            .write_xml(Parameters(WriteXmlParams {
                output_path: None,
                playlists: None,
            }))
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
    async fn write_xml_with_playlists_exports_without_staged_changes() {
        let db_conn = create_single_track_test_db("playlist-track-1", "/tmp/playlist-track-1.flac");
        let store_dir = tempfile::tempdir().expect("temp store dir should create");
        let store_path = store_dir.path().join("internal.sqlite3");
        let store_conn = store::open(
            store_path
                .to_str()
                .expect("temp store path should be UTF-8"),
        )
        .expect("temp internal store should open");
        let server =
            create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());

        let output_dir = tempfile::tempdir().expect("temp output dir should create");
        let output_path = output_dir.path().join("playlist-export.xml");
        let output_path_str = output_path.to_string_lossy().to_string();

        let result = server
            .write_xml(Parameters(WriteXmlParams {
                output_path: Some(output_path_str.clone()),
                playlists: Some(vec![WriteXmlPlaylistInput {
                    name: "Set & Test".to_string(),
                    track_ids: vec!["playlist-track-1".to_string()],
                }]),
            }))
            .await
            .expect("write_xml should export playlist-only requests");

        let payload = extract_json(&result);
        assert_eq!(payload["track_count"], 1);
        assert_eq!(payload["changes_applied"], 0);
        assert_eq!(payload["playlist_count"], 1);
        assert_eq!(
            payload["path"].as_str().expect("path should be present"),
            output_path_str
        );

        let xml = std::fs::read_to_string(&output_path).expect("XML output should be readable");
        assert!(xml.contains("<PLAYLISTS>"));
        assert!(xml.contains("Name=\"Set &amp; Test\""));
        assert!(xml.contains("<TRACK Key=\"1\"/>"));
    }

    #[tokio::test]
    async fn write_xml_with_playlists_reports_missing_track_ids() {
        let db_conn = create_single_track_test_db("playlist-track-1", "/tmp/playlist-track-1.flac");
        let store_dir = tempfile::tempdir().expect("temp store dir should create");
        let store_path = store_dir.path().join("internal.sqlite3");
        let store_conn = store::open(
            store_path
                .to_str()
                .expect("temp store path should be UTF-8"),
        )
        .expect("temp internal store should open");
        let server =
            create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());

        let err = server
            .write_xml(Parameters(WriteXmlParams {
                output_path: None,
                playlists: Some(vec![WriteXmlPlaylistInput {
                    name: "Bad Set".to_string(),
                    track_ids: vec!["does-not-exist".to_string()],
                }]),
            }))
            .await
            .expect_err("missing playlist track IDs should fail");

        let msg = format!("{err:?}");
        assert!(msg.contains("Track IDs not found in database"));
        assert!(msg.contains("does-not-exist"));
    }

    #[tokio::test]
    async fn write_xml_with_playlists_and_staged_changes_exports_union_once() {
        let db_conn = create_single_track_test_db("staged-track-1", "/tmp/staged-track-1.flac");
        insert_test_track(
            &db_conn,
            "playlist-track-2",
            "Playlist Only",
            "g1",
            "/tmp/playlist-track-2.flac",
        );

        let store_dir = tempfile::tempdir().expect("temp store dir should create");
        let store_path = store_dir.path().join("internal.sqlite3");
        let store_conn = store::open(
            store_path
                .to_str()
                .expect("temp store path should be UTF-8"),
        )
        .expect("temp internal store should open");
        let server =
            create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());

        server
            .update_tracks(Parameters(UpdateTracksParams {
                changes: vec![TrackChangeInput {
                    track_id: "staged-track-1".to_string(),
                    genre: None,
                    comments: Some("staged only comment".to_string()),
                    rating: Some(5),
                    color: None,
                }],
            }))
            .await
            .expect("staging update should succeed");

        let output_dir = tempfile::tempdir().expect("temp output dir should create");
        let output_path = output_dir.path().join("mixed-export.xml");
        let output_path_str = output_path.to_string_lossy().to_string();

        let result = server
            .write_xml(Parameters(WriteXmlParams {
                output_path: Some(output_path_str.clone()),
                playlists: Some(vec![WriteXmlPlaylistInput {
                    name: "Mixed Export".to_string(),
                    track_ids: vec!["playlist-track-2".to_string(), "staged-track-1".to_string()],
                }]),
            }))
            .await
            .expect("write_xml should succeed for mixed staged + playlist exports");

        let payload = extract_json(&result);
        assert_eq!(payload["track_count"], 2);
        assert_eq!(payload["changes_applied"], 1);
        assert_eq!(payload["playlist_count"], 1);
        assert_eq!(
            payload["path"].as_str().expect("path should be present"),
            output_path_str
        );

        let xml = std::fs::read_to_string(&output_path).expect("XML output should be readable");
        assert!(xml.contains("<COLLECTION Entries=\"2\">"));
        assert_eq!(xml.matches("<TRACK TrackID=\"").count(), 2);
        assert_eq!(xml.matches("Name=\"Señorita\"").count(), 1);
        assert_eq!(xml.matches("Name=\"Playlist Only\"").count(), 1);

        let staged_line = xml
            .lines()
            .find(|line| line.contains("Name=\"Señorita\""))
            .expect("staged track line should exist");
        assert!(
            staged_line.contains("Comments=\"staged only comment\""),
            "staged comment should be applied to staged track"
        );
        assert!(
            staged_line.contains("Rating=\"255\""),
            "5-star staged rating should be encoded as 255"
        );

        let playlist_only_line = xml
            .lines()
            .find(|line| line.contains("Name=\"Playlist Only\""))
            .expect("playlist-only track line should exist");
        assert!(
            playlist_only_line.contains("Comments=\"cache coverage test\""),
            "playlist-only track should keep DB comments when no staged changes exist"
        );
        assert!(
            playlist_only_line.contains("Rating=\"102\""),
            "playlist-only track should keep DB-derived rating when not staged"
        );

        let playlist_start = xml
            .find("<NODE Type=\"1\" Name=\"Mixed Export\" Entries=\"2\" KeyType=\"0\">")
            .expect("playlist node should exist");
        let playlist_end = playlist_start
            + xml[playlist_start..]
                .find("</NODE>")
                .expect("playlist node should close");
        let playlist_block = &xml[playlist_start..playlist_end];
        let key2 = playlist_block
            .find("<TRACK Key=\"2\"/>")
            .expect("playlist should reference playlist-only track");
        let key1 = playlist_block
            .find("<TRACK Key=\"1\"/>")
            .expect("playlist should reference staged track");
        assert!(
            key2 < key1,
            "playlist key order should follow input track_ids order"
        );
    }

    #[tokio::test]
    async fn update_tracks_includes_provenance() {
        let server = ReklawdboxServer::new(None);
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

    #[test]
    fn enrich_tracks_invalid_provider_rejected_by_serde() {
        let json = serde_json::json!({
            "providers": ["spotify"],
        });
        let result = serde_json::from_value::<EnrichTracksParams>(json);
        assert!(
            result.is_err(),
            "serde should reject unknown provider variant"
        );
    }

    #[tokio::test]
    async fn lookup_discogs_without_auth_returns_actionable_remediation() {
        if discogs::BrokerConfig::from_env().is_some() || discogs::legacy_credentials_configured() {
            eprintln!("Skipping auth-remediation test: local Discogs env is already configured");
            return;
        }

        let server = ReklawdboxServer::new(None);
        let err = server
            .lookup_discogs(Parameters(LookupDiscogsParams {
                track_id: None,
                artist: Some("No Auth Artist".to_string()),
                title: Some("No Auth Title".to_string()),
                album: None,
                force_refresh: Some(true),
            }))
            .await
            .expect_err(
                "lookup_discogs should fail with actionable remediation when auth is missing",
            );

        let msg = format!("{err}");
        assert!(
            msg.contains("Discogs auth is not configured"),
            "error should explain missing auth, got: {msg}"
        );
        assert!(
            msg.contains(discogs::BROKER_URL_ENV),
            "error should mention broker URL env var, got: {msg}"
        );
    }

    #[tokio::test]
    async fn lookup_discogs_no_match_payload_is_consistent_across_live_and_cache_paths() {
        let db_conn =
            create_single_track_test_db("discogs-no-match-track", "/tmp/discogs-no-match.flac");
        let store_dir = tempfile::tempdir().expect("temp store dir should create");
        let store_path = store_dir.path().join("internal.sqlite3");
        let store_conn = store::open(
            store_path
                .to_str()
                .expect("temp store path should be UTF-8"),
        )
        .expect("temp internal store should open");
        let server =
            create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());

        let artist = "Discogs NoMatch Artist";
        let title = "Discogs NoMatch Title";
        set_test_discogs_lookup_override(artist, title, None, Ok(None));

        let live_result = server
            .lookup_discogs(Parameters(LookupDiscogsParams {
                track_id: None,
                artist: Some(artist.to_string()),
                title: Some(title.to_string()),
                album: None,
                force_refresh: Some(true),
            }))
            .await
            .expect("live discogs no-match should succeed");
        let live_payload = extract_json(&live_result);
        assert_eq!(live_payload["result"], serde_json::Value::Null);
        assert_eq!(live_payload["cache_hit"], false);
        assert!(
            live_payload.get("cached_at").is_none(),
            "live payload should omit cached_at"
        );

        let cache_result = server
            .lookup_discogs(Parameters(LookupDiscogsParams {
                track_id: None,
                artist: Some(artist.to_string()),
                title: Some(title.to_string()),
                album: None,
                force_refresh: Some(false),
            }))
            .await
            .expect("cached discogs no-match should succeed");
        let cache_payload = extract_json(&cache_result);
        assert_eq!(cache_payload["result"], serde_json::Value::Null);
        assert_eq!(cache_payload["cache_hit"], true);

        let cache_hit_timestamp = cache_payload
            .get("cached_at")
            .and_then(serde_json::Value::as_str)
            .expect("cached no-match payload should include cached_at");
        let norm_artist = discogs::normalize(artist);
        let norm_title = discogs::normalize(title);
        let cache_entry = {
            let store = server
                .internal_conn()
                .expect("internal store should be available");
            store::get_enrichment(&store, "discogs", &norm_artist, &norm_title)
                .expect("cache read should succeed")
                .expect("discogs no-match lookup should create cache entry")
        };
        assert!(
            cache_entry.response_json.is_none(),
            "discogs no-match cache entry should store null response as no payload"
        );
        assert_eq!(
            cache_hit_timestamp,
            cache_entry.created_at.as_str(),
            "cached_at should match persisted cache timestamp"
        );
    }

    #[tokio::test]
    async fn lookup_beatport_no_match_payload_is_consistent_across_live_and_cache_paths() {
        let db_conn =
            create_single_track_test_db("beatport-no-match-track", "/tmp/beatport-no-match.flac");
        let store_dir = tempfile::tempdir().expect("temp store dir should create");
        let store_path = store_dir.path().join("internal.sqlite3");
        let store_conn = store::open(
            store_path
                .to_str()
                .expect("temp store path should be UTF-8"),
        )
        .expect("temp internal store should open");
        let server =
            create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());

        let artist = "Beatport NoMatch Artist";
        let title = "Beatport NoMatch Title";
        set_test_beatport_lookup_override(artist, title, Ok(None));

        let live_result = server
            .lookup_beatport(Parameters(LookupBeatportParams {
                track_id: None,
                artist: Some(artist.to_string()),
                title: Some(title.to_string()),
                force_refresh: Some(true),
            }))
            .await
            .expect("live beatport no-match should succeed");
        let live_payload = extract_json(&live_result);
        assert_eq!(live_payload["result"], serde_json::Value::Null);
        assert_eq!(live_payload["cache_hit"], false);
        assert!(
            live_payload.get("cached_at").is_none(),
            "live payload should omit cached_at"
        );

        let cache_result = server
            .lookup_beatport(Parameters(LookupBeatportParams {
                track_id: None,
                artist: Some(artist.to_string()),
                title: Some(title.to_string()),
                force_refresh: Some(false),
            }))
            .await
            .expect("cached beatport no-match should succeed");
        let cache_payload = extract_json(&cache_result);
        assert_eq!(cache_payload["result"], serde_json::Value::Null);
        assert_eq!(cache_payload["cache_hit"], true);

        let cache_hit_timestamp = cache_payload
            .get("cached_at")
            .and_then(serde_json::Value::as_str)
            .expect("cached no-match payload should include cached_at");
        let norm_artist = discogs::normalize(artist);
        let norm_title = discogs::normalize(title);
        let cache_entry = {
            let store = server
                .internal_conn()
                .expect("internal store should be available");
            store::get_enrichment(&store, "beatport", &norm_artist, &norm_title)
                .expect("cache read should succeed")
                .expect("beatport no-match lookup should create cache entry")
        };
        assert!(
            cache_entry.response_json.is_none(),
            "beatport no-match cache entry should store null response as no payload"
        );
        assert_eq!(
            cache_hit_timestamp,
            cache_entry.created_at.as_str(),
            "cached_at should match persisted cache timestamp"
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
            filters: SearchFilterParams::default(),
            track_ids: Some(vec![
                "cached-track-1".to_string(),
                "cached-track-2".to_string(),
            ]),
            playlist_id: None,
            max_tracks: Some(10),
            providers: Some(vec![crate::types::Provider::Discogs]),
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
                filters: SearchFilterParams::default(),
                track_ids: Some(vec![
                    "cached-track-1".to_string(),
                    "cached-track-2".to_string(),
                ]),
                playlist_id: None,
                max_tracks: Some(10),
                providers: Some(vec![crate::types::Provider::Discogs]),
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
    async fn enrich_tracks_summary_uses_provider_attempt_totals() {
        let db_conn = create_single_track_test_db("cached-track-1", "/tmp/cached-track-1.flac");
        let store_dir = tempfile::tempdir().expect("temp store dir should create");
        let store_path = store_dir.path().join("internal.sqlite3");
        let store_conn = store::open(
            store_path
                .to_str()
                .expect("temp store path should be UTF-8"),
        )
        .expect("temp internal store should open");

        let norm_artist = discogs::normalize("Aníbal");
        let norm_title = discogs::normalize("Señorita");
        store::set_enrichment(
            &store_conn,
            "discogs",
            &norm_artist,
            &norm_title,
            Some("exact"),
            Some(r#"{"styles":["Deep House"]}"#),
        )
        .expect("discogs cache should seed");
        store::set_enrichment(
            &store_conn,
            "beatport",
            &norm_artist,
            &norm_title,
            Some("exact"),
            Some(r#"{"genre":"Deep House"}"#),
        )
        .expect("beatport cache should seed");

        let server =
            create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());
        let result = server
            .enrich_tracks(Parameters(EnrichTracksParams {
                filters: SearchFilterParams::default(),
                track_ids: Some(vec!["cached-track-1".to_string()]),
                playlist_id: None,
                max_tracks: Some(1),
                providers: Some(vec![crate::types::Provider::Discogs, crate::types::Provider::Beatport]),
                skip_cached: Some(true),
                force_refresh: Some(false),
            }))
            .await
            .expect("enrich_tracks should resolve from cache for both providers");
        let payload = extract_json(&result);

        assert_eq!(payload["summary"]["tracks_total"], 1);
        assert_eq!(payload["summary"]["total"], 2);
        assert_eq!(payload["summary"]["cached"], 2);
        assert_eq!(payload["summary"]["enriched"], 0);
        assert_eq!(payload["summary"]["skipped"], 0);
        assert_eq!(payload["summary"]["failed"], 0);
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
    async fn cache_coverage_reports_provider_coverage_and_gap_counts() {
        let db_conn = create_single_track_test_db("coverage-with-genre", "/music/coverage-1.flac");
        insert_test_track(
            &db_conn,
            "coverage-no-genre-1",
            "No Genre One",
            "",
            "/music/coverage-2.flac",
        );
        insert_test_track(
            &db_conn,
            "coverage-no-genre-2",
            "No Genre Two",
            "",
            "/music/coverage-3.flac",
        );
        insert_test_track(
            &db_conn,
            "coverage-no-genre-3",
            "No Genre Three",
            "",
            "/music/coverage-4.flac",
        );

        let store_dir = tempfile::tempdir().expect("temp store dir should create");
        let store_path = store_dir.path().join("internal.sqlite3");
        let store_conn = store::open(
            store_path
                .to_str()
                .expect("temp store path should be UTF-8"),
        )
        .expect("temp internal store should open");

        let norm_artist = discogs::normalize("Aníbal");
        let norm_title_one = discogs::normalize("No Genre One");
        let norm_title_two = discogs::normalize("No Genre Two");

        store::set_audio_analysis(
            &store_conn,
            "/music/coverage-2.flac",
            "stratum-dsp",
            1234,
            1_700_000_001,
            "stratum-dsp-1.0.0",
            r#"{"bpm":127.1,"key":"Am"}"#,
        )
        .expect("stratum cache should be seeded");
        store::set_audio_analysis(
            &store_conn,
            "/music/coverage-2.flac",
            "essentia",
            1234,
            1_700_000_001,
            "essentia-2.1",
            r#"{"danceability":0.81}"#,
        )
        .expect("essentia cache should be seeded");
        store::set_enrichment(
            &store_conn,
            "discogs",
            &norm_artist,
            &norm_title_one,
            Some("exact"),
            Some(r#"{"styles":["Deep House"]}"#),
        )
        .expect("discogs cache should be seeded for first ungenred track");
        store::set_enrichment(
            &store_conn,
            "beatport",
            &norm_artist,
            &norm_title_one,
            Some("exact"),
            Some(r#"{"genre":"Deep House"}"#),
        )
        .expect("beatport cache should be seeded for first ungenred track");
        store::set_enrichment(
            &store_conn,
            "discogs",
            &norm_artist,
            &norm_title_two,
            Some("exact"),
            Some(r#"{"styles":["Tech House"]}"#),
        )
        .expect("discogs cache should be seeded for second ungenred track");

        let server =
            create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());
        server
            .state
            .essentia_python
            .set(Some("/tmp/fake-essentia-python".to_string()))
            .expect("essentia probe cache should be set exactly once");

        let result = server
            .cache_coverage(Parameters(ResolveTracksDataParams {
                filters: SearchFilterParams {
                    has_genre: Some(false),
                    ..Default::default()
                },
                track_ids: None,
                playlist_id: None,
                max_tracks: None,
            }))
            .await
            .expect("cache_coverage should succeed");
        let payload = extract_json(&result);

        assert_eq!(payload["scope"]["total_tracks"], 4);
        assert_eq!(payload["scope"]["matched_tracks"], 3);
        assert_eq!(payload["scope"]["filter_description"], "has_genre = false");

        assert_eq!(payload["coverage"]["stratum_dsp"]["cached"], 1);
        assert_eq!(payload["coverage"]["stratum_dsp"]["percent"], 33.3);

        assert_eq!(payload["coverage"]["essentia"]["cached"], 1);
        assert_eq!(payload["coverage"]["essentia"]["percent"], 33.3);
        assert_eq!(payload["coverage"]["essentia"]["installed"], true);

        assert_eq!(payload["coverage"]["discogs"]["cached"], 2);
        assert_eq!(payload["coverage"]["discogs"]["percent"], 66.7);

        assert_eq!(payload["coverage"]["beatport"]["cached"], 1);
        assert_eq!(payload["coverage"]["beatport"]["percent"], 33.3);

        assert_eq!(payload["gaps"]["no_audio_analysis"], 2);
        assert_eq!(payload["gaps"]["no_enrichment"], 1);
        assert_eq!(payload["gaps"]["no_data_at_all"], 1);
    }

    #[tokio::test]
    async fn cache_coverage_excludes_sampler_tracks_for_id_and_playlist_scopes() {
        let db_conn = create_single_track_test_db("coverage-base", "/music/coverage-base.flac");
        insert_test_track(
            &db_conn,
            "coverage-nonsample",
            "Coverage Non Sample",
            "",
            "/music/coverage-nonsample.flac",
        );
        let sampler_path = format!("{}CoverageSampler.wav", db::SAMPLER_PATH_PREFIX);
        insert_test_track(
            &db_conn,
            "coverage-sampler",
            "Coverage Sampler",
            "",
            &sampler_path,
        );

        db_conn
            .execute_batch(
                "CREATE TABLE djmdSongPlaylist (
                    PlaylistID VARCHAR(255),
                    ContentID VARCHAR(255),
                    TrackNo INTEGER
                );",
            )
            .expect("playlist table should be created for test");
        db_conn
            .execute(
                "INSERT INTO djmdSongPlaylist (PlaylistID, ContentID, TrackNo) VALUES (?1, ?2, ?3)",
                params!["pl-cache", "coverage-nonsample", 1],
            )
            .expect("non-sampler playlist entry should insert");
        db_conn
            .execute(
                "INSERT INTO djmdSongPlaylist (PlaylistID, ContentID, TrackNo) VALUES (?1, ?2, ?3)",
                params!["pl-cache", "coverage-sampler", 2],
            )
            .expect("sampler playlist entry should insert");

        let store_dir = tempfile::tempdir().expect("temp store dir should create");
        let store_path = store_dir.path().join("internal.sqlite3");
        let store_conn = store::open(
            store_path
                .to_str()
                .expect("temp store path should be UTF-8"),
        )
        .expect("temp internal store should open");
        let server =
            create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());

        let id_scope = server
            .cache_coverage(Parameters(ResolveTracksDataParams {
                filters: SearchFilterParams::default(),
                track_ids: Some(vec![
                    "coverage-nonsample".to_string(),
                    "coverage-sampler".to_string(),
                ]),
                playlist_id: None,
                max_tracks: None,
            }))
            .await
            .expect("cache_coverage track_ids scope should succeed");
        let id_payload = extract_json(&id_scope);
        assert!(id_payload["scope"]["total_tracks"].as_u64().unwrap() >= 2);
        assert_eq!(id_payload["scope"]["matched_tracks"], 1);
        assert_eq!(id_payload["gaps"]["no_data_at_all"], 1);

        let playlist_scope = server
            .cache_coverage(Parameters(ResolveTracksDataParams {
                filters: SearchFilterParams::default(),
                track_ids: None,
                playlist_id: Some("pl-cache".to_string()),
                max_tracks: None,
            }))
            .await
            .expect("cache_coverage playlist scope should succeed");
        let playlist_payload = extract_json(&playlist_scope);
        assert!(playlist_payload["scope"]["total_tracks"].as_u64().unwrap() >= 2);
        assert_eq!(playlist_payload["scope"]["matched_tracks"], 1);
        assert_eq!(playlist_payload["gaps"]["no_data_at_all"], 1);
    }

    #[tokio::test]
    #[ignore]
    async fn force_refresh_bypasses_enrichment_cache() {
        let offline_http = reqwest::Client::builder()
            .user_agent("Reklawdbox/0.1")
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
                track_id: None,
                artist: Some(track.artist.clone()),
                title: Some(track.title.clone()),
                force_refresh: Some(false),
            }))
            .await
            .expect("lookup_beatport(force_refresh=false) should return cache");
        let cache_hit_json = extract_json(&cache_hit);
        assert_eq!(cache_hit_json["cache_hit"], true);
        assert_eq!(cache_hit_json["genre"], "Sentinel Genre");

        let refresh_err = server
            .lookup_beatport(Parameters(LookupBeatportParams {
                track_id: None,
                artist: Some(track.artist.clone()),
                title: Some(track.title.clone()),
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
    async fn enrich_tracks_beatport_schema_matches_individual_lookup() {
        let Some((server, _store_dir)) =
            create_real_server_with_temp_store(default_http_client_for_tests())
        else {
            eprintln!("Skipping: backup tarball not found (set REKORDBOX_TEST_BACKUP)");
            return;
        };

        let candidates = sample_real_tracks(&server, 30);
        if candidates.is_empty() {
            eprintln!("Skipping: integration test needs candidate tracks from real DB");
            return;
        }

        let mut selected_track: Option<crate::types::Track> = None;
        for track in candidates.into_iter().take(10) {
            let lookup = server
                .lookup_beatport(Parameters(LookupBeatportParams {
                    track_id: None,
                    artist: Some(track.artist.clone()),
                    title: Some(track.title.clone()),
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

        let Some(track) = selected_track else {
            eprintln!(
                "Skipping: could not find a track with a successful Beatport match; \
                 rerun when network/providers are available"
            );
            return;
        };
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
                filters: SearchFilterParams::default(),
                track_ids: Some(vec![track.id.clone()]),
                playlist_id: None,
                max_tracks: Some(1),
                providers: Some(vec![crate::types::Provider::Beatport]),
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
                filters: SearchFilterParams::default(),
                track_ids: Some(track_ids.clone()),
                playlist_id: None,
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
            file_type_name: "FLAC File".to_string(),
            date_added: "2023-01-15".to_string(),
            position: None,
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

    #[test]
    fn standard_key_to_camelot_converts_major_minor_and_flats() {
        assert_eq!(
            standard_key_to_camelot("Am").map(format_camelot).as_deref(),
            Some("8A")
        );
        assert_eq!(
            standard_key_to_camelot("C").map(format_camelot).as_deref(),
            Some("8B")
        );
        assert_eq!(
            standard_key_to_camelot("F#m")
                .map(format_camelot)
                .as_deref(),
            Some("11A")
        );
        assert_eq!(
            standard_key_to_camelot("Bb").map(format_camelot).as_deref(),
            Some("6B")
        );
        assert_eq!(
            standard_key_to_camelot("Dbm")
                .map(format_camelot)
                .as_deref(),
            Some("12A")
        );
        assert_eq!(
            key_to_camelot("8a").map(format_camelot).as_deref(),
            Some("8A")
        );
        assert_eq!(standard_key_to_camelot("not-a-key"), None);
    }

    #[test]
    fn camelot_distance_scoring_handles_wrap_and_mode_shift() {
        let wrap_up = score_key_axis(parse_camelot_key("12A"), parse_camelot_key("1A"));
        assert_eq!(wrap_up.value, 0.9);
        assert!(
            wrap_up.label.contains("Energy boost"),
            "wrap-around up should be treated as +1"
        );

        let wrap_down = score_key_axis(parse_camelot_key("1A"), parse_camelot_key("12A"));
        assert_eq!(wrap_down.value, 0.9);
        assert!(
            wrap_down.label.contains("Energy drop"),
            "wrap-around down should be treated as -1"
        );

        let mood_shift = score_key_axis(parse_camelot_key("6A"), parse_camelot_key("6B"));
        assert_eq!(mood_shift.value, 0.8);

        let rough = score_key_axis(parse_camelot_key("6A"), parse_camelot_key("7B"));
        assert_eq!(rough.value, 0.4);
    }

    #[test]
    fn composite_scoring_changes_by_priority_axis() {
        let approx = |left: f64, right: f64| (left - right).abs() < 1e-9;

        assert!(approx(
            composite_score(
                1.0,
                0.0,
                0.0,
                0.0,
                Some(0.0),
                Some(0.0),
                SetPriority::Balanced
            ),
            0.30
        ));
        assert!(approx(
            composite_score(
                1.0,
                0.0,
                0.0,
                0.0,
                Some(0.0),
                Some(0.0),
                SetPriority::Harmonic
            ),
            0.48
        ));
        assert!(approx(
            composite_score(
                1.0,
                0.0,
                0.0,
                0.0,
                Some(0.0),
                Some(0.0),
                SetPriority::Energy
            ),
            0.12
        ));
        assert!(approx(
            composite_score(1.0, 0.0, 0.0, 0.0, Some(0.0), Some(0.0), SetPriority::Genre),
            0.18
        ));

        assert!(approx(
            composite_score(
                0.0,
                0.0,
                0.0,
                1.0,
                Some(0.0),
                Some(0.0),
                SetPriority::Balanced
            ),
            0.17
        ));
        assert!(approx(
            composite_score(0.0, 0.0, 0.0, 1.0, Some(0.0), Some(0.0), SetPriority::Genre),
            0.38
        ));

        assert!(approx(
            composite_score(1.0, 0.0, 0.0, 0.0, None, None, SetPriority::Balanced),
            0.30 / 0.85
        ));
    }

    #[test]
    fn score_genre_axis_treats_missing_genre_as_neutral() {
        let unknown_source =
            score_genre_axis(None, Some("House"), GenreFamily::Other, GenreFamily::House);
        assert_eq!(unknown_source.value, 0.5);
        assert_eq!(unknown_source.label, "Unknown genre");

        let unknown_destination =
            score_genre_axis(Some("House"), None, GenreFamily::House, GenreFamily::Other);
        assert_eq!(unknown_destination.value, 0.5);
        assert_eq!(unknown_destination.label, "Unknown genre");
    }

    #[test]
    fn bpm_proxy_energy_keeps_peak_phase_reachable_without_essentia() {
        let from_energy = compute_track_energy(None, 126.0);
        let to_energy = compute_track_energy(None, 130.0);
        let peak = score_energy_axis(
            from_energy,
            to_energy,
            Some(EnergyPhase::Peak),
            Some(EnergyPhase::Peak),
            None,
        );

        assert!(
            to_energy >= 0.65,
            "fallback energy should allow peak thresholds"
        );
        assert_eq!(peak.value, 1.0);
        assert_eq!(peak.label, "High and stable (peak phase)");
    }

    #[tokio::test]
    async fn score_transition_returns_expected_axis_scores() {
        let db_conn = create_single_track_test_db("from-track", "/tmp/from-track.flac");
        db_conn
            .execute(
                "INSERT INTO djmdKey (ID, ScaleName) VALUES ('k2', 'Em')",
                [],
            )
            .expect("second key should insert");
        db_conn
            .execute(
                "INSERT INTO djmdContent (
                    ID, Title, ArtistID, AlbumID, GenreID, KeyID, ColorID, LabelID, RemixerID,
                    BPM, Rating, Commnt, ReleaseYear, Length, FolderPath, DJPlayCount, BitRate,
                    SampleRate, FileType, created_at, rb_local_deleted
                ) VALUES (
                    ?1, 'Second Track', 'a1', 'al1', 'g1', 'k2', 'c1', 'l1', '',
                    12350, 153, 'score transition test', 2025, 260, ?2, '0', 1411,
                    44100, 5, '2025-01-03', 0
                )",
                params!["to-track", "/tmp/to-track.flac"],
            )
            .expect("second track should insert");

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
            "/tmp/from-track.flac",
            "stratum-dsp",
            1,
            1,
            "stratum-dsp-1.0.0",
            r#"{"bpm":122.0,"key":"Am","key_camelot":"8A"}"#,
        )
        .expect("source stratum cache should seed");
        store::set_audio_analysis(
            &store_conn,
            "/tmp/to-track.flac",
            "stratum-dsp",
            1,
            1,
            "stratum-dsp-1.0.0",
            r#"{"bpm":123.5,"key":"Em","key_camelot":"9A"}"#,
        )
        .expect("destination stratum cache should seed");

        store::set_audio_analysis(
            &store_conn,
            "/tmp/from-track.flac",
            "essentia",
            1,
            1,
            "essentia-2.1",
            r#"{"danceability":0.90,"loudness_integrated":-12.0,"onset_rate":3.0}"#,
        )
        .expect("source essentia cache should seed");
        store::set_audio_analysis(
            &store_conn,
            "/tmp/to-track.flac",
            "essentia",
            1,
            1,
            "essentia-2.1",
            r#"{"danceability":1.80,"loudness_integrated":-8.0,"onset_rate":5.0}"#,
        )
        .expect("destination essentia cache should seed");

        let server =
            create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());
        let result = server
            .score_transition(Parameters(ScoreTransitionParams {
                from_track_id: "from-track".to_string(),
                to_track_id: "to-track".to_string(),
                energy_phase: Some(EnergyPhase::Build),
                priority: Some(SetPriority::Balanced),
            }))
            .await
            .expect("score_transition should succeed");

        let payload = extract_json(&result);
        assert_eq!(payload["from"]["track_id"], "from-track");
        assert_eq!(payload["from"]["key"], "8A");
        assert_eq!(payload["to"]["track_id"], "to-track");
        assert_eq!(payload["to"]["key"], "9A");

        assert_eq!(payload["scores"]["key"]["value"], 0.9);
        assert_eq!(payload["scores"]["bpm"]["value"], 1.0);
        assert_eq!(payload["scores"]["energy"]["value"], 1.0);
        assert_eq!(payload["scores"]["genre"]["value"], 1.0);
        assert_eq!(payload["scores"]["brightness"]["value"], 0.5);
        assert_eq!(payload["scores"]["rhythm"]["value"], 0.5);
        assert_eq!(payload["scores"]["composite"], 0.965);
    }

    // ==================== serde/schema contract tests for #[serde(flatten)] ====================

    /// Verify that flat JSON (as sent by MCP) deserializes correctly into all
    /// param structs that use `#[serde(flatten)] filters: SearchFilterParams`.
    /// Guards against regressions where fields silently stop binding.
    #[test]
    fn flatten_json_round_trip_search_tracks_params() {
        let json = serde_json::json!({
            "query": "burial",
            "artist": "Burial",
            "genre": "Dubstep",
            "rating_min": 3,
            "bpm_min": 130.0,
            "bpm_max": 145.0,
            "key": "Am",
            "has_genre": true,
            "label": "Hyperdub",
            "path": "/Music",
            "added_after": "2026-01-01",
            "added_before": "2026-12-31",
            "playlist": "p1",
            "include_samples": true,
            "limit": 50,
            "offset": 10,
        });
        let p: SearchTracksParams = serde_json::from_value(json).expect("should deserialize");
        assert_eq!(p.filters.query.as_deref(), Some("burial"));
        assert_eq!(p.filters.artist.as_deref(), Some("Burial"));
        assert_eq!(p.filters.genre.as_deref(), Some("Dubstep"));
        assert_eq!(p.filters.rating_min, Some(3));
        assert_eq!(p.filters.bpm_min, Some(130.0));
        assert_eq!(p.filters.bpm_max, Some(145.0));
        assert_eq!(p.filters.key.as_deref(), Some("Am"));
        assert_eq!(p.filters.has_genre, Some(true));
        assert_eq!(p.filters.label.as_deref(), Some("Hyperdub"));
        assert_eq!(p.filters.path.as_deref(), Some("/Music"));
        assert_eq!(p.filters.added_after.as_deref(), Some("2026-01-01"));
        assert_eq!(p.filters.added_before.as_deref(), Some("2026-12-31"));
        assert_eq!(p.playlist.as_deref(), Some("p1"));
        assert_eq!(p.include_samples, Some(true));
        assert_eq!(p.limit, Some(50));
        assert_eq!(p.offset, Some(10));
    }

    #[test]
    fn flatten_json_round_trip_enrich_tracks_params() {
        let json = serde_json::json!({
            "genre": "Techno",
            "bpm_min": 125.0,
            "track_ids": ["t1", "t2"],
            "playlist_id": "p1",
            "max_tracks": 20,
            "providers": ["discogs", "beatport"],
            "skip_cached": false,
            "force_refresh": true,
        });
        let p: EnrichTracksParams = serde_json::from_value(json).expect("should deserialize");
        assert_eq!(p.filters.genre.as_deref(), Some("Techno"));
        assert_eq!(p.filters.bpm_min, Some(125.0));
        assert_eq!(p.filters.query, None);
        assert_eq!(p.track_ids.as_ref().unwrap().len(), 2);
        assert_eq!(p.playlist_id.as_deref(), Some("p1"));
        assert_eq!(p.max_tracks, Some(20));
        assert_eq!(p.skip_cached, Some(false));
        assert_eq!(p.force_refresh, Some(true));
    }

    #[test]
    fn flatten_json_round_trip_analyze_audio_batch_params() {
        let json = serde_json::json!({
            "artist": "Aphex Twin",
            "rating_min": 4,
            "track_ids": ["t1"],
            "max_tracks": 10,
            "skip_cached": true,
        });
        let p: AnalyzeAudioBatchParams = serde_json::from_value(json).expect("should deserialize");
        assert_eq!(p.filters.artist.as_deref(), Some("Aphex Twin"));
        assert_eq!(p.filters.rating_min, Some(4));
        assert_eq!(p.track_ids.as_ref().unwrap(), &["t1"]);
        assert_eq!(p.max_tracks, Some(10));
        assert_eq!(p.skip_cached, Some(true));
    }

    #[test]
    fn flatten_json_round_trip_resolve_tracks_data_params() {
        let json = serde_json::json!({
            "key": "Cm",
            "has_genre": false,
            "added_after": "2025-06-01",
            "playlist_id": "p2",
            "max_tracks": 100,
        });
        let p: ResolveTracksDataParams = serde_json::from_value(json).expect("should deserialize");
        assert_eq!(p.filters.key.as_deref(), Some("Cm"));
        assert_eq!(p.filters.has_genre, Some(false));
        assert_eq!(p.filters.added_after.as_deref(), Some("2025-06-01"));
        assert_eq!(p.playlist_id.as_deref(), Some("p2"));
        assert_eq!(p.max_tracks, Some(100));
    }

    #[test]
    fn flatten_json_empty_payload_deserializes_to_all_none() {
        let json = serde_json::json!({});
        let p: SearchTracksParams = serde_json::from_value(json.clone()).expect("SearchTracksParams");
        assert!(p.filters.query.is_none());
        assert!(p.playlist.is_none());
        assert!(p.limit.is_none());

        let p: EnrichTracksParams = serde_json::from_value(json.clone()).expect("EnrichTracksParams");
        assert!(p.filters.genre.is_none());
        assert!(p.track_ids.is_none());

        let p: AnalyzeAudioBatchParams = serde_json::from_value(json.clone()).expect("AnalyzeAudioBatchParams");
        assert!(p.filters.artist.is_none());
        assert!(p.track_ids.is_none());

        let p: ResolveTracksDataParams = serde_json::from_value(json).expect("ResolveTracksDataParams");
        assert!(p.filters.key.is_none());
        assert!(p.track_ids.is_none());
    }

    /// Verify that schemars inlines flattened fields at the top level of the
    /// JSON Schema. MCP clients read the schema to build tool UIs — a nested
    /// `filters` wrapper object would break them.
    #[test]
    fn flatten_schema_has_top_level_filter_properties() {
        // Filter fields that must appear as top-level properties in every schema
        let filter_fields = [
            "query", "artist", "genre", "rating_min", "bpm_min", "bpm_max",
            "key", "has_genre", "label", "path", "added_after", "added_before",
        ];

        fn assert_schema_properties<T: JsonSchema>(type_name: &str, expected: &[&str], forbidden: &[&str]) {
            let schema = schemars::schema_for!(T);
            let root = schema.as_value();
            let props = root.get("properties")
                .expect(&format!("{type_name} schema should have properties"));
            for field in expected {
                assert!(
                    props.get(*field).is_some(),
                    "{type_name} schema missing top-level property '{field}'"
                );
            }
            for field in forbidden {
                assert!(
                    props.get(*field).is_none(),
                    "{type_name} schema should NOT have property '{field}'"
                );
            }
        }

        // SearchTracksParams: filter fields + playlist, include_samples, limit, offset
        assert_schema_properties::<SearchTracksParams>(
            "SearchTracksParams",
            &[&filter_fields[..], &["playlist", "include_samples", "limit", "offset"]].concat(),
            &["filters"],
        );

        // EnrichTracksParams: filter fields + track_ids, playlist_id, max_tracks, providers, skip_cached, force_refresh
        assert_schema_properties::<EnrichTracksParams>(
            "EnrichTracksParams",
            &[&filter_fields[..], &["track_ids", "playlist_id", "max_tracks", "providers", "skip_cached", "force_refresh"]].concat(),
            &["filters"],
        );

        // AnalyzeAudioBatchParams: filter fields + track_ids, playlist_id, max_tracks, skip_cached
        assert_schema_properties::<AnalyzeAudioBatchParams>(
            "AnalyzeAudioBatchParams",
            &[&filter_fields[..], &["track_ids", "playlist_id", "max_tracks", "skip_cached"]].concat(),
            &["filters"],
        );

        // ResolveTracksDataParams: filter fields + track_ids, playlist_id, max_tracks
        assert_schema_properties::<ResolveTracksDataParams>(
            "ResolveTracksDataParams",
            &[&filter_fields[..], &["track_ids", "playlist_id", "max_tracks"]].concat(),
            &["filters"],
        );
    }
}
