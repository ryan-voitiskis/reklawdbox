use std::process::Stdio;

use rmcp::ErrorData as McpError;
use rmcp::model::{CallToolResult, Content};

use super::*;
use crate::audio;
use crate::db;
use crate::store;

pub(super) async fn handle_analyze_track_audio(
    server: &ReklawdboxServer,
    params: AnalyzeTrackAudioParams,
) -> Result<CallToolResult, McpError> {
    let skip_cached = params.skip_cached.unwrap_or(true);

    let track = {
        let conn = server.rekordbox_conn()?;
        db::get_track(&conn, &params.track_id)
            .map_err(|e| mcp_internal_error(format!("DB error: {e}")))?
            .ok_or_else(|| {
                McpError::invalid_params(format!("Track '{}' not found", params.track_id), None)
            })?
    };

    let file_path = resolve_file_path(&track.file_path)?;
    let metadata = std::fs::metadata(&file_path)
        .map_err(|e| mcp_internal_error(format!("Cannot stat file '{}': {e}", file_path)))?;
    let file_size = metadata.len() as i64;
    let file_mtime = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    // Stratum-dsp: check cache then analyze
    let stratum_cached = if skip_cached {
        let store = server.cache_store_conn()?;
        check_analysis_cache(
            &store,
            &file_path,
            audio::ANALYZER_STRATUM,
            file_size,
            file_mtime,
        )
        .map_err(mcp_internal_error)?
    } else {
        None
    };

    let (stratum_dsp, stratum_cache_hit) = if let Some(json_str) = stratum_cached {
        let val = serde_json::from_str(&json_str)
            .map_err(|e| mcp_internal_error(format!("Cache parse error: {e}")))?;
        (val, true)
    } else {
        let analysis = analyze_stratum(&file_path)
            .await
            .map_err(mcp_internal_error)?;
        let features_json =
            serde_json::to_string(&analysis).map_err(|e| mcp_internal_error(format!("{e}")))?;
        let store = server.cache_store_conn()?;
        store::set_audio_analysis(
            &store,
            &file_path,
            audio::ANALYZER_STRATUM,
            file_size,
            file_mtime,
            &analysis.analyzer_version,
            &features_json,
        )
        .map_err(|e| mcp_internal_error(format!("Cache write error: {e}")))?;
        (
            serde_json::to_value(&analysis).map_err(|e| mcp_internal_error(format!("{e}")))?,
            false,
        )
    };

    // Essentia: check cache then analyze
    let essentia_python = server.essentia_python_path();
    let essentia_available = essentia_python.is_some();
    let mut essentia: Option<serde_json::Value> = None;
    let mut essentia_cache_hit: Option<bool> = None;
    let mut essentia_error: Option<String> = None;

    if let Some(python_path) = essentia_python.as_deref() {
        let essentia_cached = if skip_cached {
            let store = server.cache_store_conn()?;
            check_analysis_cache(
                &store,
                &file_path,
                audio::ANALYZER_ESSENTIA,
                file_size,
                file_mtime,
            )
            .map_err(mcp_internal_error)?
        } else {
            None
        };

        if let Some(json_str) = essentia_cached {
            essentia = Some(
                serde_json::from_str(&json_str)
                    .map_err(|e| mcp_internal_error(format!("Cache parse error: {e}")))?,
            );
            essentia_cache_hit = Some(true);
        } else {
            match audio::run_essentia(python_path, &file_path)
                .await
                .map_err(|e| e.to_string())
            {
                Ok(features) => {
                    let version = if features.analyzer_version.is_empty() {
                        "unknown"
                    } else {
                        &features.analyzer_version
                    };
                    let features_json = serde_json::to_string(&features)
                        .map_err(|e| mcp_internal_error(format!("{e}")))?;
                    let store = server.cache_store_conn()?;
                    store::set_audio_analysis(
                        &store,
                        &file_path,
                        audio::ANALYZER_ESSENTIA,
                        file_size,
                        file_mtime,
                        version,
                        &features_json,
                    )
                    .map_err(|e| mcp_internal_error(format!("Cache write error: {e}")))?;
                    essentia = Some(
                        serde_json::to_value(&features)
                            .map_err(|e| mcp_internal_error(format!("{e}")))?,
                    );
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
    let json =
        serde_json::to_string_pretty(&result).map_err(|e| mcp_internal_error(format!("{e}")))?;
    Ok(CallToolResult::success(vec![Content::text(json)]))
}

pub(super) struct BatchTrackAnalysis {
    pub(super) track_id: String,
    pub(super) title: String,
    pub(super) artist: String,
    pub(super) file_path: String,
    pub(super) file_size: i64,
    pub(super) file_mtime: i64,
    pub(super) stratum_dsp: serde_json::Value,
    pub(super) stratum_cache_hit: bool,
    pub(super) essentia: Option<serde_json::Value>,
    pub(super) essentia_cache_hit: Option<bool>,
    pub(super) essentia_error: Option<String>,
}

pub(super) async fn handle_analyze_audio_batch(
    server: &ReklawdboxServer,
    params: AnalyzeAudioBatchParams,
) -> Result<CallToolResult, McpError> {
    let skip_cached = params.skip_cached.unwrap_or(true);

    let tracks = {
        let conn = server.rekordbox_conn()?;
        resolve_tracks(
            &conn,
            params.track_ids.as_deref(),
            params.playlist_id.as_deref(),
            params.filters,
            params.max_tracks,
            &ResolveTracksOpts {
                default_max_tracks: Some(20),
                max_tracks_cap: Some(200),
                exclude_samplers: false,
            },
        )?
    };

    let total = tracks.len();

    let mut progress = BatchProgress::new();
    let mut essentia_analyzed = 0usize;
    let mut essentia_cached = 0usize;
    let mut essentia_failed = 0usize;
    let mut rows: Vec<BatchTrackAnalysis> = Vec::new();

    for track in &tracks {
        let file_path = match resolve_file_path(&track.file_path) {
            Ok(path) => path,
            Err(e) => {
                progress.failures.push(serde_json::json!({
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
                progress.failures.push(serde_json::json!({
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
            let store = server.cache_store_conn()?;
            match check_analysis_cache(
                &store,
                &file_path,
                audio::ANALYZER_STRATUM,
                file_size,
                file_mtime,
            ) {
                Ok(Some(json_str)) => match serde_json::from_str(&json_str) {
                    Ok(val) => {
                        stratum_dsp = Some(val);
                        stratum_cache_hit = true;
                        progress.cached += 1;
                    }
                    Err(e) => {
                        progress.failures.push(serde_json::json!({
                            "track_id": track.id, "artist": track.artist,
                            "title": track.title, "analyzer": audio::ANALYZER_STRATUM,
                            "error": format!("Cache parse error: {e}"),
                        }));
                        continue;
                    }
                },
                Ok(None) => {}
                Err(e) => {
                    progress.failures.push(serde_json::json!({
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
                    let features_json = serde_json::to_string(&analysis)
                        .map_err(|e| mcp_internal_error(format!("{e}")))?;
                    let store = server.cache_store_conn()?;
                    store::set_audio_analysis(
                        &store,
                        &file_path,
                        audio::ANALYZER_STRATUM,
                        file_size,
                        file_mtime,
                        &analysis.analyzer_version,
                        &features_json,
                    )
                    .map_err(|e| mcp_internal_error(format!("Cache write error: {e}")))?;
                    stratum_dsp = Some(
                        serde_json::to_value(&analysis)
                            .map_err(|e| mcp_internal_error(format!("{e}")))?,
                    );
                    progress.processed += 1;
                }
                Err(e) => {
                    progress.failures.push(serde_json::json!({
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
            stratum_dsp: stratum_dsp.ok_or_else(|| {
                mcp_internal_error("Missing stratum-dsp result in batch".to_string())
            })?,
            stratum_cache_hit,
            essentia: None,
            essentia_cache_hit: None,
            essentia_error: None,
        });
    }

    // Essentia pass
    let essentia_python = server.essentia_python_path();
    let essentia_available = essentia_python.is_some();

    if let Some(python_path) = essentia_python.as_deref() {
        for row in &mut rows {
            if skip_cached {
                let store = server.cache_store_conn()?;
                match check_analysis_cache(
                    &store,
                    &row.file_path,
                    audio::ANALYZER_ESSENTIA,
                    row.file_size,
                    row.file_mtime,
                ) {
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
                            progress.failures.push(serde_json::json!({
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
                        progress.failures.push(serde_json::json!({
                            "track_id": &row.track_id, "artist": &row.artist,
                            "title": &row.title, "analyzer": audio::ANALYZER_ESSENTIA, "error": e,
                        }));
                        continue;
                    }
                }
            }

            match audio::run_essentia(python_path, &row.file_path)
                .await
                .map_err(|e| e.to_string())
            {
                Ok(features) => {
                    let version = if features.analyzer_version.is_empty() {
                        "unknown"
                    } else {
                        &features.analyzer_version
                    };
                    let features_json = serde_json::to_string(&features)
                        .map_err(|e| mcp_internal_error(format!("{e}")))?;
                    let store = server.cache_store_conn()?;
                    store::set_audio_analysis(
                        &store,
                        &row.file_path,
                        audio::ANALYZER_ESSENTIA,
                        row.file_size,
                        row.file_mtime,
                        version,
                        &features_json,
                    )
                    .map_err(|e| mcp_internal_error(format!("Cache write error: {e}")))?;
                    row.essentia = Some(
                        serde_json::to_value(&features)
                            .map_err(|e| mcp_internal_error(format!("{e}")))?,
                    );
                    row.essentia_cache_hit = Some(false);
                    essentia_analyzed += 1;
                }
                Err(e) => {
                    row.essentia_error = Some(e.clone());
                    essentia_failed += 1;
                    progress.failures.push(serde_json::json!({
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
            "analyzed": progress.processed,
            "cached": progress.cached,
            "failed": progress.failures.len(),
            "essentia_available": essentia_available,
            "essentia_analyzed": essentia_analyzed,
            "essentia_cached": essentia_cached,
            "essentia_failed": essentia_failed,
        },
        "results": results,
        "failures": progress.failures,
    });
    if !essentia_available {
        result["essentia_setup_hint"] = serde_json::Value::String(essentia_setup_hint());
    }
    let json =
        serde_json::to_string_pretty(&result).map_err(|e| mcp_internal_error(format!("{e}")))?;
    Ok(CallToolResult::success(vec![Content::text(json)]))
}

pub(super) async fn handle_setup_essentia(
    server: &ReklawdboxServer,
) -> Result<CallToolResult, McpError> {
    // Serialize concurrent setup calls - only one install at a time
    let _setup_guard = server.state.essentia_setup_lock.lock().await;

    // Check if already available (validate to catch stale overrides)
    if let Some(path) = server.essentia_python_path() {
        if validate_essentia_python(&path) {
            let result = serde_json::json!({
                "status": "already_installed",
                "python_path": path,
                "message": "Essentia is already available.",
            });
            let json = serde_json::to_string_pretty(&result)
                .map_err(|e| mcp_internal_error(format!("{e}")))?;
            return Ok(CallToolResult::success(vec![Content::text(json)]));
        }
        // Stale override - clear it and proceed with fresh install
        if let Ok(mut guard) = server.state.essentia_python_override.lock() {
            *guard = None;
        }
    }

    let venv_dir = essentia_venv_dir().ok_or_else(|| {
        mcp_internal_error("Cannot determine home directory for venv location".to_string())
    })?;

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
                mcp_internal_error(format!(
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
        .map_err(|e| mcp_internal_error(format!("venv task failed: {e}")))?
        .map_err(|e| mcp_internal_error(format!("Failed to run {python_bin} -m venv: {e}")))?;

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
        .map_err(|e| mcp_internal_error(format!("pip task failed: {e}")))?
        .map_err(|e| mcp_internal_error(format!("Failed to run pip install: {e}")))?;

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
        .map_err(|e| mcp_internal_error(format!("validate task failed: {e}")))?
        .map_err(|e| {
            mcp_internal_error(format!("Failed to validate essentia installation: {e}"))
        })?;

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
        let mut guard = server
            .state
            .essentia_python_override
            .lock()
            .map_err(|_| mcp_internal_error("essentia override lock poisoned".to_string()))?;
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
        let json = serde_json::to_string_pretty(&result)
            .map_err(|e| mcp_internal_error(format!("{e}")))?;
        return Ok(CallToolResult::success(vec![Content::text(json)]));
    }

    Err(mcp_internal_error(format!(
        "All Python candidates failed. Last error: {last_error}"
    )))
}
