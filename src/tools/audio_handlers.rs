use std::process::Stdio;

use rmcp::ErrorData as McpError;
use rmcp::model::{CallToolResult, Content};

use super::*;
use crate::audio;
use crate::db;
use crate::store;

fn file_mtime_secs(metadata: &std::fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub(super) async fn handle_analyze_track_audio(
    server: &ReklawdboxServer,
    params: AnalyzeTrackAudioParams,
) -> Result<CallToolResult, McpError> {
    let skip_cached = params.skip_cached.unwrap_or(true);

    let track = {
        let conn = server.rekordbox_conn()?;
        db::get_track(&conn, &params.track_id)
            .map_err(db_error)?
            .ok_or_else(|| {
                McpError::invalid_params(format!("Track '{}' not found", params.track_id), None)
            })?
    };

    let file_path = resolve_file_path(&track.file_path)?;

    // Stratum-dsp: check cache then analyze
    let stratum_cached = if skip_cached {
        let store = server.cache_store_conn()?;
        check_analysis_cache(
            &store,
            &file_path,
            audio::ANALYZER_STRATUM,
            audio::STRATUM_SCHEMA_VERSION,
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
        let metadata = tokio::fs::metadata(&file_path)
            .await
            .map_err(|e| mcp_internal_error(format!("Cannot stat file '{}': {e}", file_path)))?;
        let store = server.cache_store_conn()?;
        store::set_audio_analysis(
            &store,
            &file_path,
            audio::ANALYZER_STRATUM,
            metadata.len() as i64,
            file_mtime_secs(&metadata),
            audio::STRATUM_SCHEMA_VERSION,
            &features_json,
        )
        .map_err(cache_error)?;
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
                audio::ESSENTIA_SCHEMA_VERSION,
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
                    let features_json = serde_json::to_string(&features)
                        .map_err(|e| mcp_internal_error(format!("{e}")))?;
                    let metadata = tokio::fs::metadata(&file_path)
                        .await
                        .map_err(|e| mcp_internal_error(format!("Cannot stat file: {e}")))?;
                    let store = server.cache_store_conn()?;
                    store::set_audio_analysis(
                        &store,
                        &file_path,
                        audio::ANALYZER_ESSENTIA,
                        metadata.len() as i64,
                        file_mtime_secs(&metadata),
                        audio::ESSENTIA_SCHEMA_VERSION,
                        &features_json,
                    )
                    .map_err(cache_error)?;
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
    pub(super) stratum_dsp: serde_json::Value,
    pub(super) stratum_cache_hit: bool,
    pub(super) essentia: Option<serde_json::Value>,
    pub(super) essentia_cache_hit: Option<bool>,
    pub(super) essentia_error: Option<String>,
}

enum CacheWriteMsg {
    Audio {
        file_path: String,
        analyzer: &'static str,
        file_size: i64,
        file_mtime: i64,
        analyzer_version: String,
        features_json: String,
    },
}

#[allow(clippy::too_many_arguments)]
async fn analyze_single_track(
    track_id: String,
    title: String,
    artist: String,
    raw_file_path: String,
    skip_cached: bool,
    essentia_python: Option<String>,
    store_path: String,
    cache_tx: tokio::sync::mpsc::Sender<CacheWriteMsg>,
) -> Result<BatchTrackAnalysis, serde_json::Value> {
    // Resolve file path
    let file_path = audio::resolve_audio_path(&raw_file_path).map_err(|e| {
        serde_json::json!({
            "track_id": &track_id, "artist": &artist, "title": &title,
            "analyzer": audio::ANALYZER_STRATUM,
            "error": format!("File path error: {e}"),
        })
    })?;

    // Open read-only cache connection for this task
    let cache_conn = store::open_read_only(&store_path).map_err(|e| {
        serde_json::json!({
            "track_id": &track_id, "artist": &artist, "title": &title,
            "analyzer": audio::ANALYZER_STRATUM,
            "error": format!("Cache open error: {e}"),
        })
    })?;

    // Check stratum cache
    let stratum_cached = if skip_cached {
        check_analysis_cache(
            &cache_conn,
            &file_path,
            audio::ANALYZER_STRATUM,
            audio::STRATUM_SCHEMA_VERSION,
        )
        .ok()
        .flatten()
    } else {
        None
    };

    // Check essentia cache
    let essentia_cached = if skip_cached && essentia_python.is_some() {
        check_analysis_cache(
            &cache_conn,
            &file_path,
            audio::ANALYZER_ESSENTIA,
            audio::ESSENTIA_SCHEMA_VERSION,
        )
        .ok()
        .flatten()
    } else {
        None
    };

    // Drop the read connection before running analysis
    drop(cache_conn);

    // Run uncached analyzers in parallel via tokio::join!
    let stratum_fut = async {
        if let Some(json_str) = &stratum_cached {
            let val: serde_json::Value =
                serde_json::from_str(json_str).map_err(|e| format!("Cache parse error: {e}"))?;
            return Ok::<(serde_json::Value, bool), String>((val, true));
        }
        let analysis = analyze_stratum(&file_path).await?;
        let features_json =
            serde_json::to_string(&analysis).map_err(|e| format!("Serialize error: {e}"))?;
        let val = serde_json::to_value(&analysis).map_err(|e| format!("Serialize error: {e}"))?;
        let metadata = tokio::fs::metadata(&file_path)
            .await
            .map_err(|e| format!("Cannot stat file: {e}"))?;
        cache_tx
            .send(CacheWriteMsg::Audio {
                file_path: file_path.clone(),
                analyzer: audio::ANALYZER_STRATUM,
                file_size: metadata.len() as i64,
                file_mtime: file_mtime_secs(&metadata),
                analyzer_version: audio::STRATUM_SCHEMA_VERSION.to_string(),
                features_json,
            })
            .await
            .map_err(|e| format!("Cache write queue error: {e}"))?;
        Ok((val, false))
    };

    let essentia_python_clone = essentia_python.clone();
    let file_path_clone = file_path.clone();
    let cache_tx_clone = cache_tx.clone();
    let essentia_fut = async {
        let python_path = match &essentia_python_clone {
            Some(p) => p,
            None => return (None, None, None), // essentia not available
        };
        if let Some(json_str) = &essentia_cached {
            match serde_json::from_str(json_str) {
                Ok(val) => return (Some(val), Some(true), None),
                Err(e) => return (None, None, Some(format!("Cache parse error: {e}"))),
            }
        }
        match audio::run_essentia(python_path, &file_path_clone)
            .await
            .map_err(|e| e.to_string())
        {
            Ok(features) => {
                let features_json = match serde_json::to_string(&features) {
                    Ok(j) => j,
                    Err(e) => return (None, None, Some(format!("Serialize error: {e}"))),
                };
                let val = match serde_json::to_value(&features) {
                    Ok(v) => v,
                    Err(e) => return (None, None, Some(format!("Serialize error: {e}"))),
                };
                let metadata = match tokio::fs::metadata(&file_path_clone).await {
                    Ok(m) => m,
                    Err(e) => return (None, None, Some(format!("Cannot stat file: {e}"))),
                };
                if let Err(e) = cache_tx_clone
                    .send(CacheWriteMsg::Audio {
                        file_path: file_path_clone.clone(),
                        analyzer: audio::ANALYZER_ESSENTIA,
                        file_size: metadata.len() as i64,
                        file_mtime: file_mtime_secs(&metadata),
                        analyzer_version: audio::ESSENTIA_SCHEMA_VERSION.to_string(),
                        features_json,
                    })
                    .await
                {
                    return (None, None, Some(format!("Cache write queue error: {e}")));
                }
                (Some(val), Some(false), None)
            }
            Err(e) => (None, None, Some(e)),
        }
    };

    let (stratum_result, (essentia_val, essentia_cache_hit, essentia_error)) =
        tokio::join!(stratum_fut, essentia_fut);

    let (stratum_dsp, stratum_cache_hit) = stratum_result.map_err(|e| {
        serde_json::json!({
            "track_id": &track_id, "artist": &artist, "title": &title,
            "analyzer": audio::ANALYZER_STRATUM, "error": e,
        })
    })?;

    Ok(BatchTrackAnalysis {
        track_id,
        title,
        artist,
        stratum_dsp,
        stratum_cache_hit,
        essentia: essentia_val,
        essentia_cache_hit,
        essentia_error,
    })
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
            params.offset,
            &ResolveTracksOpts {
                default_max_tracks: Some(20),
                max_tracks_cap: Some(200),
                exclude_samplers: false,
            },
        )?
    };

    let total = tracks.len();

    // Compute concurrency
    let concurrency = match params.concurrency {
        Some(n) => n.clamp(1, 4),
        None => {
            let cpus = std::thread::available_parallelism()
                .map(|n| n.get() as u32)
                .unwrap_or(4);
            (cpus.saturating_sub(2)).clamp(1, 4)
        }
    } as usize;

    let essentia_python = server.essentia_python_path();
    let essentia_available = essentia_python.is_some();
    let store_path = server.cache_store_path();

    // Ensure the DB exists and is migrated before spawning readers
    {
        let _conn = server.cache_store_conn()?;
    }

    // Spawn cache writer task
    let (cache_tx, mut cache_rx) = tokio::sync::mpsc::channel::<CacheWriteMsg>(concurrency * 4);
    let writer_store_path = store_path.clone();
    let writer_handle = tokio::task::spawn_blocking(move || -> Result<(), String> {
        let conn = match store::open(&writer_store_path) {
            Ok(c) => c,
            Err(e) => {
                let msg = format!("Cache writer failed to open store: {e}");
                tracing::error!("{msg}");
                return Err(msg);
            }
        };
        while let Some(msg) = cache_rx.blocking_recv() {
            match msg {
                CacheWriteMsg::Audio {
                    file_path,
                    analyzer,
                    file_size,
                    file_mtime,
                    analyzer_version,
                    features_json,
                } => {
                    if let Err(e) = store::set_audio_analysis(
                        &conn,
                        &file_path,
                        analyzer,
                        file_size,
                        file_mtime,
                        &analyzer_version,
                        &features_json,
                    ) {
                        tracing::error!(
                            "Cache writer: failed to write {analyzer} for {file_path}: {e}"
                        );
                        return Err(format!(
                            "Cache writer failed to write {analyzer} for {file_path}: {e}"
                        ));
                    }
                }
            }
        }
        Ok(())
    });

    // Spawn analysis tasks bounded by semaphore
    let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(concurrency));
    let mut handles = Vec::with_capacity(total);

    for track in &tracks {
        let permit = sem
            .clone()
            .acquire_owned()
            .await
            .map_err(|e| mcp_internal_error(format!("Semaphore error: {e}")))?;
        let track_id = track.id.clone();
        let title = track.title.clone();
        let artist = track.artist.clone();
        let raw_file_path = track.file_path.clone();
        let essentia_python = essentia_python.clone();
        let store_path = store_path.clone();
        let cache_tx = cache_tx.clone();

        handles.push(tokio::spawn(async move {
            let result = analyze_single_track(
                track_id,
                title,
                artist,
                raw_file_path,
                skip_cached,
                essentia_python,
                store_path,
                cache_tx,
            )
            .await;
            drop(permit);
            result
        }));
    }

    // Collect results in order
    let mut progress = BatchProgress::new();
    let mut essentia_analyzed = 0usize;
    let mut essentia_cached = 0usize;
    let mut essentia_failed = 0usize;
    let mut rows: Vec<BatchTrackAnalysis> = Vec::new();

    for handle in handles {
        match handle.await {
            Ok(Ok(row)) => {
                if row.stratum_cache_hit {
                    progress.cached += 1;
                } else {
                    progress.processed += 1;
                }
                match row.essentia_cache_hit {
                    Some(true) => essentia_cached += 1,
                    Some(false) => essentia_analyzed += 1,
                    None if row.essentia_error.is_some() => essentia_failed += 1,
                    _ => {}
                }
                if let Some(ref err) = row.essentia_error {
                    progress.failures.push(serde_json::json!({
                        "track_id": &row.track_id, "artist": &row.artist,
                        "title": &row.title, "analyzer": audio::ANALYZER_ESSENTIA,
                        "error": err,
                    }));
                }
                rows.push(row);
            }
            Ok(Err(failure)) => {
                progress.failures.push(failure);
            }
            Err(e) => {
                progress.failures.push(serde_json::json!({
                    "error": format!("Task panicked: {e}"),
                }));
            }
        }
    }

    // Shut down writer
    drop(cache_tx);
    match writer_handle.await {
        Ok(Ok(())) => {}
        Ok(Err(err)) => progress
            .failures
            .push(serde_json::json!({ "analyzer": "cache_writer", "error": err })),
        Err(err) => progress.failures.push(
            serde_json::json!({ "analyzer": "cache_writer", "error": format!("Cache writer task failed: {err}") }),
        ),
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
            "concurrency": concurrency,
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
        let path_clone = path.clone();
        let is_valid = match tokio::task::spawn_blocking(move || {
            validate_essentia_python(&path_clone)
        })
        .await
        {
            Ok(valid) => valid,
            Err(e) => {
                tracing::warn!("Essentia validation task failed: {e}");
                false
            }
        };
        if is_valid {
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
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
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
