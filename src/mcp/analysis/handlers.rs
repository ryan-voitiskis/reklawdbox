use std::process::Stdio;

use rmcp::ErrorData as McpError;
use rmcp::model::CallToolResult;

use crate::adapters::audio::{
    self, ESSENTIA_IMPORT_CHECK_SCRIPT, essentia_setup_hint, essentia_venv_dir,
    validate_essentia_python,
};
use crate::adapters::rekordbox as db;
use crate::adapters::state as store;
use crate::application::analysis::identity::{
    AudioCacheIdentity, audio_cache_identities_with_current_stratum_input, check_analysis_cache,
};
use crate::application::analysis::{batch as analysis_batch, job as analysis_job};
use crate::mcp::{
    AnalyzeAudioBatchParams, AnalyzeTrackAudioParams, BatchPage, BatchProgress, ReklawdboxServer,
    cache_error, db_error, mcp_internal_error, ok_json, ok_structured_json, resolve_file_path,
    resolve_pending_tracks,
};

pub(in crate::mcp) async fn handle_analyze_track_audio(
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
        let report = analysis_job::run(&file_path, true, false, None, false)
            .await
            .map_err(mcp_internal_error)?;
        let analysis = report
            .stratum
            .ok_or_else(|| mcp_internal_error("Stratum analysis did not run"))?
            .map_err(mcp_internal_error)?;
        let val = serde_json::to_value(&analysis).map_err(|e| mcp_internal_error(e.to_string()))?;
        for message in report.cache_messages {
            let store = server.cache_store_conn()?;
            analysis_batch::persist_analysis_cache_write(&store, &message).map_err(cache_error)?;
        }
        (val, false)
    };

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
            let report = analysis_job::run(&file_path, false, true, Some(python_path), false)
                .await
                .map_err(mcp_internal_error)?;
            match report.essentia.expect("requested Essentia result") {
                Ok(features) => {
                    let val = serde_json::to_value(&features)
                        .map_err(|e| mcp_internal_error(e.to_string()))?;
                    for message in report.cache_messages {
                        let store = server.cache_store_conn()?;
                        analysis_batch::persist_analysis_cache_write(&store, &message)
                            .map_err(cache_error)?;
                    }
                    essentia = Some(val);
                    essentia_cache_hit = Some(false);
                }
                Err(error) => {
                    let prefix = format!("Essentia error for {file_path}: ");
                    essentia_error =
                        Some(error.strip_prefix(&prefix).unwrap_or(&error).to_string());
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
    ok_json(&result)
}

pub(in crate::mcp) struct BatchTrackAnalysis {
    pub(in crate::mcp) track_id: String,
    pub(in crate::mcp) title: String,
    pub(in crate::mcp) artist: String,
    pub(in crate::mcp) stratum_dsp: Option<serde_json::Value>,
    pub(in crate::mcp) stratum_cache_hit: bool,
    pub(in crate::mcp) stratum_error: Option<String>,
    pub(in crate::mcp) stratum_cache_write_error: Option<String>,
    pub(in crate::mcp) essentia: Option<serde_json::Value>,
    pub(in crate::mcp) essentia_cache_hit: Option<bool>,
    pub(in crate::mcp) essentia_error: Option<String>,
    pub(in crate::mcp) essentia_cache_write_error: Option<String>,
}

fn audio_completion_flags(
    store_conn: &rusqlite::Connection,
    tracks: &[crate::domain::library::Track],
    essentia_required: bool,
) -> Result<Vec<bool>, McpError> {
    let identities = audio_cache_identities_with_current_stratum_input(
        tracks.iter().map(|track| track.file_path.as_str()),
    );
    let stratum_identities: Vec<_> = identities
        .iter()
        .filter_map(|identity| identity.as_ref()?.as_stratum_store_identity())
        .collect();
    let fresh_stratum = store::batch_fresh_audio_analysis_existence(
        store_conn,
        &stratum_identities,
        audio::ANALYZER_STRATUM,
        audio::STRATUM_SCHEMA_VERSION,
    )
    .map_err(cache_error)?;

    let fresh_essentia = if essentia_required {
        let essentia_identities: Vec<_> = identities
            .iter()
            .flatten()
            .map(AudioCacheIdentity::as_essentia_store_identity)
            .collect();
        store::batch_fresh_audio_analysis_existence(
            store_conn,
            &essentia_identities,
            audio::ANALYZER_ESSENTIA,
            audio::ESSENTIA_SCHEMA_VERSION,
        )
        .map_err(cache_error)?
    } else {
        std::collections::HashSet::new()
    };

    Ok(identities
        .iter()
        .map(|identity| {
            identity.as_ref().is_some_and(|identity| {
                fresh_stratum.contains(&identity.cache_key)
                    && (!essentia_required || fresh_essentia.contains(&identity.cache_key))
            })
        })
        .collect())
}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub(in crate::mcp) struct AudioBatchSummary {
    total: usize,
    analyzed: usize,
    cached: usize,
    failed: usize,
    essentia_available: bool,
    essentia_analyzed: usize,
    essentia_cached: usize,
    essentia_failed: usize,
    concurrency: usize,
}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub(in crate::mcp) struct AudioBatchResult {
    track_id: String,
    title: String,
    artist: String,
    stratum_dsp: serde_json::Value,
    stratum_cache_hit: bool,
    essentia: Option<serde_json::Value>,
    essentia_cache_hit: Option<bool>,
    essentia_available: bool,
    essentia_error: Option<String>,
}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub(in crate::mcp) struct AnalyzeAudioBatchOutput {
    summary: AudioBatchSummary,
    results: Vec<AudioBatchResult>,
    failures: Vec<serde_json::Value>,
    page: BatchPage,
    #[serde(skip_serializing_if = "Option::is_none")]
    essentia_setup_hint: Option<String>,
}

fn audio_page_summary_counts(
    page: &BatchPage,
    selected_stratum_cached: usize,
    selected_essentia_cached: usize,
    essentia_available: bool,
) -> (usize, usize, usize) {
    let fully_current = page.fully_cached_skipped;
    let cached = selected_stratum_cached.saturating_add(fully_current);
    let essentia_cached =
        selected_essentia_cached.saturating_add(if essentia_available { fully_current } else { 0 });
    (page.examined_tracks, cached, essentia_cached)
}

fn audio_join_failures(
    track_id: &str,
    artist: &str,
    title: &str,
    essentia_available: bool,
    stage: &str,
    error: &str,
) -> Vec<serde_json::Value> {
    std::iter::once(audio::ANALYZER_STRATUM)
        .chain(essentia_available.then_some(audio::ANALYZER_ESSENTIA))
        .map(|analyzer| {
            serde_json::json!({
                "track_id": track_id,
                "artist": artist,
                "title": title,
                "analyzer": analyzer,
                "stage": stage,
                "error": error,
            })
        })
        .collect()
}

fn audio_cache_write_failure(
    track_id: &str,
    artist: &str,
    title: &str,
    analyzer: &str,
    error: &str,
) -> serde_json::Value {
    serde_json::json!({
        "track_id": track_id,
        "artist": artist,
        "title": title,
        "analyzer": analyzer,
        "stage": if error.contains("store open failed") {
            "cache_writer_open"
        } else {
            "cache_write"
        },
        "error": error,
    })
}

type CacheWriteMsg = crate::application::analysis::model::AnalysisCacheWrite;
type AudioCacheWriteRequest = analysis_batch::CacheWriteRequest<CacheWriteMsg>;

#[allow(clippy::too_many_arguments)]
async fn analyze_single_track(
    track_id: String,
    title: String,
    artist: String,
    raw_file_path: String,
    skip_cached: bool,
    essentia_python: Option<String>,
    store_path: String,
    cache_tx: tokio::sync::mpsc::Sender<AudioCacheWriteRequest>,
) -> Result<BatchTrackAnalysis, serde_json::Value> {
    let file_path = audio::resolve_audio_path(&raw_file_path).map_err(|error| {
        serde_json::json!({
            "track_id": &track_id, "artist": &artist, "title": &title,
            "analyzer": audio::ANALYZER_STRATUM,
            "stage": "resolve_file",
            "error": format!("File path error: {error}"),
        })
    })?;

    let cache_conn = store::open_read_only(&store_path).map_err(|error| {
        serde_json::json!({
            "track_id": &track_id, "artist": &artist, "title": &title,
            "analyzer": audio::ANALYZER_STRATUM,
            "stage": "cache_read",
            "error": format!("Cache open error: {error}"),
        })
    })?;

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
    drop(cache_conn);

    let needs_stratum = stratum_cached.is_none();
    let needs_essentia = essentia_python.is_some() && essentia_cached.is_none();
    let mut report = if needs_stratum || needs_essentia {
        analysis_job::run(
            &file_path,
            needs_stratum,
            needs_essentia,
            essentia_python.as_deref(),
            true,
        )
        .await
        .map_err(|error| {
            serde_json::json!({
                "track_id": &track_id, "artist": &artist, "title": &title,
                "analyzer": audio::ANALYZER_STRATUM,
                "stage": "analysis",
                "error": error,
            })
        })?
    } else {
        analysis_job::AnalysisJobReport {
            stratum: None,
            essentia: None,
            cache_messages: Vec::new(),
            elapsed_seconds: 0.0,
        }
    };

    let mut stratum_cache_write_error = None;
    let mut essentia_cache_write_error = None;
    for message in report.cache_messages.drain(..) {
        let analyzer = message.analyzer.clone();
        if let Err(error) =
            analysis_batch::send_cache_message(&cache_tx, message, &format!("{analyzer} analysis"))
                .await
        {
            if analyzer == audio::ANALYZER_STRATUM {
                stratum_cache_write_error = Some(error);
            } else {
                essentia_cache_write_error = Some(error);
            }
        }
    }

    let (stratum_dsp, stratum_cache_hit, stratum_error) = if let Some(json) = stratum_cached {
        match serde_json::from_str(&json) {
            Ok(value) => (Some(value), true, None),
            Err(error) => (None, true, Some(format!("Cache parse error: {error}"))),
        }
    } else {
        match report.stratum.take().expect("requested Stratum result") {
            Ok(result) => match serde_json::to_value(result) {
                Ok(value) => (Some(value), false, None),
                Err(error) => (None, false, Some(format!("Serialize error: {error}"))),
            },
            Err(error) => (None, false, Some(error)),
        }
    };

    let (essentia, essentia_cache_hit, essentia_error) = if let Some(json) = essentia_cached {
        match serde_json::from_str(&json) {
            Ok(value) => (Some(value), Some(true), None),
            Err(error) => (None, None, Some(format!("Cache parse error: {error}"))),
        }
    } else if needs_essentia {
        match report.essentia.take().expect("requested Essentia result") {
            Ok(result) => {
                let value = serde_json::to_value(result).map_err(|error| {
                    serde_json::json!({
                        "track_id": &track_id, "artist": &artist, "title": &title,
                        "analyzer": audio::ANALYZER_ESSENTIA,
                        "stage": "analysis",
                        "error": format!("Serialize error: {error}"),
                    })
                })?;
                (Some(value), Some(false), None)
            }
            Err(error) => (None, None, Some(error)),
        }
    } else {
        (None, None, None)
    };

    Ok(BatchTrackAnalysis {
        track_id,
        title,
        artist,
        stratum_dsp,
        stratum_cache_hit,
        stratum_error,
        stratum_cache_write_error,
        essentia,
        essentia_cache_hit,
        essentia_error,
        essentia_cache_write_error,
    })
}

pub(in crate::mcp) async fn handle_analyze_audio_batch(
    server: &ReklawdboxServer,
    params: AnalyzeAudioBatchParams,
) -> Result<CallToolResult, McpError> {
    let skip_cached = params.skip_cached.unwrap_or(true);

    let essentia_python = server.essentia_python_path();
    let essentia_available = essentia_python.is_some();
    let store_path = server.cache_store_path();

    // Initialize/migrate the store, then release its MutexGuard. The pending
    // scan uses a dedicated read-only connection so it never nests the two
    // server database locks.
    {
        let _store_guard = server.cache_store_conn()?;
    }

    let selection = {
        let store_conn = if skip_cached {
            Some(store::open_read_only(&store_path).map_err(cache_error)?)
        } else {
            None
        };
        let conn = server.rekordbox_conn()?;
        if let Some(store_conn) = store_conn.as_ref() {
            resolve_pending_tracks(
                &conn,
                params.track_ids.as_deref(),
                params.playlist_id.as_deref(),
                params.filters,
                params.max_tracks,
                params.offset,
                20,
                200,
                false,
                |tracks| audio_completion_flags(store_conn, tracks, essentia_available),
            )?
        } else {
            resolve_pending_tracks(
                &conn,
                params.track_ids.as_deref(),
                params.playlist_id.as_deref(),
                params.filters,
                params.max_tracks,
                params.offset,
                20,
                200,
                false,
                |tracks| Ok(vec![false; tracks.len()]),
            )?
        }
    };
    let tracks = selection.selected;
    let page = selection.page;

    let selected_tracks = tracks.len();

    let concurrency = match params.concurrency {
        Some(n) => n.clamp(1, 4),
        None => {
            let cpus = std::thread::available_parallelism()
                .map(|n| n.get() as u32)
                .unwrap_or(4);
            (cpus.saturating_sub(2)).clamp(1, 4)
        }
    } as usize;

    let (cache_tx, cache_rx) =
        tokio::sync::mpsc::channel::<AudioCacheWriteRequest>(concurrency * 4);
    let writer_store_path = store_path.clone();
    let writer_cancel = tokio_util::sync::CancellationToken::new();
    let writer_handle = tokio::task::spawn_blocking(move || {
        analysis_batch::run_analysis_cache_writer(writer_store_path, cache_rx, writer_cancel)
    });

    let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(concurrency));
    let mut handles = Vec::with_capacity(selected_tracks);

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

        let failure_track_id = track_id.clone();
        let failure_artist = artist.clone();
        let failure_title = title.clone();
        let handle = tokio::spawn(async move {
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
        });
        handles.push((failure_track_id, failure_artist, failure_title, handle));
    }

    let mut progress = BatchProgress::new();
    let mut essentia_analyzed = 0usize;
    let mut essentia_cached = 0usize;
    let mut essentia_failed = 0usize;
    let mut rows: Vec<BatchTrackAnalysis> = Vec::new();

    for (track_id, artist, title, handle) in handles {
        match handle.await {
            Ok(Ok(row)) => {
                if row.stratum_dsp.is_some() {
                    if row.stratum_cache_hit {
                        progress.cached += 1;
                    } else {
                        progress.processed += 1;
                    }
                }
                if let Some(ref error) = row.stratum_error {
                    progress.failures.push(serde_json::json!({
                        "track_id": &row.track_id, "artist": &row.artist,
                        "title": &row.title, "analyzer": audio::ANALYZER_STRATUM,
                        "stage": "analysis", "error": error,
                    }));
                }
                if let Some(ref error) = row.stratum_cache_write_error {
                    progress.failures.push(audio_cache_write_failure(
                        &row.track_id,
                        &row.artist,
                        &row.title,
                        audio::ANALYZER_STRATUM,
                        error,
                    ));
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
                        "stage": "analysis",
                        "error": err,
                    }));
                }
                if let Some(ref error) = row.essentia_cache_write_error {
                    progress.failures.push(audio_cache_write_failure(
                        &row.track_id,
                        &row.artist,
                        &row.title,
                        audio::ANALYZER_ESSENTIA,
                        error,
                    ));
                }
                if row.stratum_dsp.is_some() {
                    rows.push(row);
                }
            }
            Ok(Err(failure)) => {
                progress.failures.push(failure);
            }
            Err(e) => {
                progress.failures.extend(audio_join_failures(
                    &track_id,
                    &artist,
                    &title,
                    essentia_available,
                    "task_join",
                    &format!("Task panicked: {e}"),
                ));
            }
        }
    }

    drop(cache_tx);
    match writer_handle.await {
        Ok(_report) => {}
        Err(err) => {
            for track in &tracks {
                progress.failures.extend(audio_join_failures(
                    &track.id,
                    &track.artist,
                    &track.title,
                    essentia_available,
                    "cache_writer_join",
                    &format!("Cache writer task failed: {err}"),
                ));
            }
        }
    }

    let results: Vec<AudioBatchResult> = rows
        .into_iter()
        .map(|row| AudioBatchResult {
            track_id: row.track_id,
            title: row.title,
            artist: row.artist,
            stratum_dsp: row
                .stratum_dsp
                .expect("successful rows contain Stratum output"),
            stratum_cache_hit: row.stratum_cache_hit,
            essentia: row.essentia,
            essentia_cache_hit: row.essentia_cache_hit,
            essentia_available,
            essentia_error: row.essentia_error,
        })
        .collect();

    let (total, cached, essentia_cached) =
        audio_page_summary_counts(&page, progress.cached, essentia_cached, essentia_available);
    let result = AnalyzeAudioBatchOutput {
        summary: AudioBatchSummary {
            total,
            analyzed: progress.processed,
            cached,
            failed: progress.failures.len(),
            essentia_available,
            essentia_analyzed,
            essentia_cached,
            essentia_failed,
            concurrency,
        },
        results,
        failures: progress.failures,
        page,
        essentia_setup_hint: (!essentia_available).then(essentia_setup_hint),
    };
    ok_structured_json(result)
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod pending_page_tests {
    use super::*;
    use crate::mcp::enrichment::pending_batch_page;

    fn track(id: &str, path: String) -> crate::domain::library::Track {
        crate::domain::library::Track {
            id: id.to_string(),
            title: id.to_string(),
            artist: "Test Artist".to_string(),
            album: String::new(),
            genre: String::new(),
            bpm: 0.0,
            key: String::new(),
            rating: 0,
            comments: String::new(),
            color: String::new(),
            color_code: 0,
            label: String::new(),
            remixer: String::new(),
            year: 0,
            length: 0,
            file_path: path,
            play_count: 0,
            bit_rate: 0,
            sample_rate: 0,
            file_kind: crate::domain::library::FileKind::Wav,
            date_added: String::new(),
            position: None,
            played_at: None,
        }
    }

    fn store() -> (tempfile::TempDir, rusqlite::Connection) {
        let dir = tempfile::tempdir().expect("temporary store directory should create");
        let path = dir.path().join("store.sqlite3");
        let conn = crate::adapters::state::open(path.to_str().expect("store path should be UTF-8"))
            .expect("temporary store should open");
        (dir, conn)
    }

    fn seed_current(conn: &rusqlite::Connection, raw_path: &str, analyzer: &str, version: &str) {
        let identity = audio_cache_identities_with_current_stratum_input([raw_path])
            .into_iter()
            .next()
            .flatten()
            .expect("audio identity should resolve");
        let input_fingerprint = if analyzer == audio::ANALYZER_STRATUM {
            identity
                .stratum_input_fingerprint
                .as_deref()
                .expect("stratum identity should include a fingerprint")
        } else {
            ""
        };
        store::set_audio_analysis_with_fingerprint(
            conn,
            &identity.cache_key,
            analyzer,
            identity.file_size,
            identity.file_mtime,
            version,
            input_fingerprint,
            "{}",
        )
        .expect("audio cache fixture should write");
    }

    #[test]
    fn analyze_audio_batch_pending_page_skips_current_and_reaches_stale_work() {
        let files = tempfile::tempdir().expect("temporary audio directory should create");
        let current_path = files.path().join("current.wav");
        let stale_path = files.path().join("stale.wav");
        std::fs::write(&current_path, b"current").expect("current fixture should write");
        std::fs::write(&stale_path, b"stale").expect("stale fixture should write");
        let missing_path = files.path().join("missing.wav");
        let tracks = vec![
            track("current", current_path.display().to_string()),
            track("stale", stale_path.display().to_string()),
            track("missing", missing_path.display().to_string()),
        ];
        let (_store_dir, conn) = store();
        seed_current(
            &conn,
            &tracks[0].file_path,
            audio::ANALYZER_STRATUM,
            audio::STRATUM_SCHEMA_VERSION,
        );
        seed_current(
            &conn,
            &tracks[1].file_path,
            audio::ANALYZER_STRATUM,
            "stale-schema",
        );

        let complete = audio_completion_flags(&conn, &tracks, false)
            .expect("audio completion lookup should succeed");
        assert_eq!(complete, [true, false, false]);
        let selection = pending_batch_page(&tracks, 0, 1, |_| Ok(complete.clone()))
            .expect("audio pending page should resolve");
        assert_eq!(selection.selected[0].id, "stale");
        assert_eq!(
            selection.page,
            BatchPage {
                matched_tracks: 3,
                start_offset: 0,
                examined_tracks: 2,
                selected_tracks: 1,
                fully_cached_skipped: 1,
                next_offset: Some(2),
                has_more: true,
            }
        );
        assert_eq!(
            audio_page_summary_counts(&selection.page, 0, 0, false),
            (2, 1, 0),
            "the fully current prefix should remain visible in the page summary"
        );

        let continuation = pending_batch_page(&tracks, 2, 1, |_| Ok(vec![false]))
            .expect("missing-file continuation should resolve");
        assert_eq!(continuation.selected[0].id, "missing");
        assert!(!continuation.page.has_more);
    }

    #[test]
    fn analyze_audio_batch_pending_page_only_requires_essentia_when_available() {
        let files = tempfile::tempdir().expect("temporary audio directory should create");
        let path = files.path().join("track.wav");
        std::fs::write(&path, b"audio").expect("audio fixture should write");
        let tracks = vec![track("track", path.display().to_string())];
        let (_store_dir, conn) = store();
        seed_current(
            &conn,
            &tracks[0].file_path,
            audio::ANALYZER_STRATUM,
            audio::STRATUM_SCHEMA_VERSION,
        );

        assert_eq!(
            audio_completion_flags(&conn, &tracks, false)
                .expect("stratum-only completion should resolve"),
            [true]
        );
        let stratum_only_page = pending_batch_page(&tracks, 0, 1, |_| Ok(vec![true]))
            .expect("stratum-only completed page should resolve");
        assert_eq!(
            audio_page_summary_counts(&stratum_only_page.page, 0, 0, false),
            (1, 1, 0)
        );
        assert_eq!(
            audio_completion_flags(&conn, &tracks, true)
                .expect("essentia-required completion should resolve"),
            [false]
        );
        let essentia_pending_page = pending_batch_page(&tracks, 0, 1, |_| Ok(vec![false]))
            .expect("Essentia-pending page should resolve");
        assert_eq!(
            audio_page_summary_counts(&essentia_pending_page.page, 1, 0, true),
            (1, 1, 0),
            "a selected track's current Stratum result should remain counted"
        );

        seed_current(
            &conn,
            &tracks[0].file_path,
            audio::ANALYZER_ESSENTIA,
            audio::ESSENTIA_SCHEMA_VERSION,
        );
        assert_eq!(
            audio_completion_flags(&conn, &tracks, true)
                .expect("dual-analyzer completion should resolve"),
            [true]
        );
        let dual_current_page = pending_batch_page(&tracks, 0, 1, |_| Ok(vec![true]))
            .expect("dual-current page should resolve");
        assert_eq!(
            audio_page_summary_counts(&dual_current_page.page, 0, 0, true),
            (1, 1, 1)
        );

        let zero_page = pending_batch_page(&tracks, 0, 0, |_| {
            panic!("zero work cap must not inspect cache state")
        })
        .expect("zero-cap page should resolve");
        assert_eq!(
            audio_page_summary_counts(&zero_page.page, 0, 0, true),
            (0, 0, 0)
        );
    }

    #[tokio::test]
    async fn analyze_audio_batch_pending_page_writer_failure_retains_retry_identity() {
        const STEP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);
        const TEST_WATCHDOG: std::time::Duration = std::time::Duration::from_secs(5);

        tokio::time::timeout(TEST_WATCHDOG, async {
            let directory = tempfile::tempdir().expect("temporary directory should create");
            let track_id = "retry-track";
            let artist = "Retry Artist";
            let title = "Retry Title";
            let analyzer = audio::ANALYZER_STRATUM;
            let (cache_tx, cache_rx) = tokio::sync::mpsc::channel(1);
            let message = CacheWriteMsg {
                file_path: "/tmp/retry.wav".to_string(),
                analyzer: analyzer.to_string(),
                file_size: 1,
                file_mtime: 2,
                analyzer_version: audio::STRATUM_SCHEMA_VERSION.to_string(),
                input_fingerprint: audio::STRATUM_HMM_INPUT_FINGERPRINT.to_string(),
                features_json: "{}".to_string(),
            };
            let writer_path = directory.path().to_string_lossy().to_string();
            let cancel = tokio_util::sync::CancellationToken::new();
            let mut writer = tokio::task::spawn_blocking(move || {
                analysis_batch::run_analysis_cache_writer(writer_path, cache_rx, cancel)
            });
            let acknowledged = tokio::time::timeout(
                STEP_TIMEOUT,
                analysis_batch::send_cache_message(&cache_tx, message, "stratum-dsp analysis"),
            )
            .await;
            let error = match acknowledged {
                Ok(result) => result.expect_err("cache writer should reject an invalid store path"),
                Err(_) => {
                    drop(cache_tx);
                    writer.abort();
                    let _ = tokio::time::timeout(STEP_TIMEOUT, &mut writer).await;
                    panic!("cache acknowledgement timed out");
                }
            };
            drop(cache_tx);
            let report = match tokio::time::timeout(STEP_TIMEOUT, &mut writer).await {
                Ok(result) => result.expect("cache writer task should join"),
                Err(_) => {
                    writer.abort();
                    let _ = tokio::time::timeout(STEP_TIMEOUT, &mut writer).await;
                    panic!("cache writer join timed out");
                }
            };
            assert_eq!(report.failed, 1);

            let failure = audio_cache_write_failure(track_id, artist, title, analyzer, &error);
            assert_eq!(failure["track_id"], track_id);
            assert_eq!(failure["artist"], artist);
            assert_eq!(failure["title"], title);
            assert_eq!(failure["analyzer"], analyzer);
            assert_eq!(failure["stage"], "cache_writer_open");
            assert_eq!(failure["error"], error);

            let ordinary_failure = audio_cache_write_failure(
                track_id,
                artist,
                title,
                analyzer,
                "injected write rejection",
            );
            assert_eq!(ordinary_failure["stage"], "cache_write");
        })
        .await
        .expect("MCP writer failure presentation scenario timed out");
    }

    #[test]
    fn analyze_audio_batch_pending_page_completion_policy_change_requires_restart() {
        let files = tempfile::tempdir().expect("temporary audio directory should create");
        let first_path = files.path().join("first.wav");
        let second_path = files.path().join("second.wav");
        std::fs::write(&first_path, b"first").expect("first fixture should write");
        std::fs::write(&second_path, b"second").expect("second fixture should write");
        let tracks = vec![
            track("first", first_path.display().to_string()),
            track("second", second_path.display().to_string()),
        ];
        let (_store_dir, conn) = store();
        for track in &tracks {
            seed_current(
                &conn,
                &track.file_path,
                audio::ANALYZER_STRATUM,
                audio::STRATUM_SCHEMA_VERSION,
            );
        }
        seed_current(
            &conn,
            &tracks[1].file_path,
            audio::ANALYZER_ESSENTIA,
            audio::ESSENTIA_SCHEMA_VERSION,
        );

        assert_eq!(
            audio_completion_flags(&conn, &tracks, false)
                .expect("stratum-only completion should resolve"),
            [true, true]
        );
        let with_essentia = audio_completion_flags(&conn, &tracks, true)
            .expect("Essentia-aware completion should resolve");
        assert_eq!(with_essentia, [false, true]);

        let completion = |candidates: &[crate::domain::library::Track]| {
            Ok(candidates
                .iter()
                .map(|track| with_essentia[usize::from(track.id == "second")])
                .collect())
        };
        let stale_offset = pending_batch_page(&tracks, 1, 1, completion)
            .expect("changed-availability offset should resolve");
        assert!(stale_offset.selected.is_empty());
        let restarted = pending_batch_page(&tracks, 0, 1, completion)
            .expect("changed-availability restart should resolve");
        assert_eq!(restarted.selected[0].id, "first");

        let skip_cached_disabled = pending_batch_page(&tracks, 0, 1, |candidates| {
            Ok(vec![false; candidates.len()])
        })
        .expect("skip-cached policy restart should resolve");
        assert_eq!(skip_cached_disabled.selected[0].id, "first");
    }

    #[test]
    fn analyze_audio_batch_pending_page_join_failure_enumerates_analyzer_identities() {
        let failures = audio_join_failures(
            "retry-track",
            "Retry Artist",
            "Retry Title",
            true,
            "cache_writer_join",
            "sentinel join failure",
        );
        assert_eq!(failures.len(), 2);
        assert_eq!(failures[0]["track_id"], "retry-track");
        assert_eq!(failures[0]["analyzer"], audio::ANALYZER_STRATUM);
        assert_eq!(failures[1]["analyzer"], audio::ANALYZER_ESSENTIA);
        assert!(
            failures
                .iter()
                .all(|failure| failure["stage"] == "cache_writer_join")
        );

        let stratum_only = audio_join_failures(
            "retry-track",
            "Retry Artist",
            "Retry Title",
            false,
            "task_join",
            "sentinel task failure",
        );
        assert_eq!(stratum_only.len(), 1);
        assert_eq!(stratum_only[0]["analyzer"], audio::ANALYZER_STRATUM);
    }
}

pub(in crate::mcp) async fn handle_setup_essentia(
    server: &ReklawdboxServer,
) -> Result<CallToolResult, McpError> {
    // Serialize concurrent setup calls - only one install at a time
    let _setup_guard = server.context.analysis.essentia_setup_lock.lock().await;

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
            return ok_json(&result);
        }
        // Stale override - clear it and proceed with fresh install
        if let Ok(mut guard) = server.context.analysis.essentia_python_override.lock() {
            *guard = None;
        }
    }

    let venv_dir = essentia_venv_dir().ok_or_else(|| {
        mcp_internal_error("Cannot determine home directory for venv location".to_string())
    })?;

    // Try each Python candidate, falling through to the next on failure
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

        // Available immediately without restart
        let mut guard = server
            .context
            .analysis
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
        return ok_json(&result);
    }

    Err(mcp_internal_error(format!(
        "All Python candidates failed. Last error: {last_error}"
    )))
}
