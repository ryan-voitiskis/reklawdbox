use rmcp::ErrorData as McpError;
use rmcp::model::{CallToolResult, Content};

use super::*;
use crate::beatport;
use crate::db;
use crate::store;

pub(super) async fn handle_lookup_discogs(
    server: &ReklawdboxServer,
    params: LookupDiscogsParams,
) -> Result<CallToolResult, McpError> {
    let force_refresh = params.force_refresh.unwrap_or(false);

    // Resolve artist/title/album: from track_id or explicit params
    let (artist, title, album) = if let Some(ref track_id) = params.track_id {
        let conn = server.rekordbox_conn()?;
        let track = db::get_track(&conn, track_id)
            .map_err(|e| mcp_internal_error(format!("DB error: {e}")))?
            .ok_or_else(|| {
                McpError::invalid_params(format!("Track '{track_id}' not found"), None)
            })?;
        let album = params
            .album
            .or_else(|| (!track.album.is_empty()).then(|| track.album.clone()));
        (
            params.artist.unwrap_or(track.artist),
            params.title.unwrap_or(track.title),
            album,
        )
    } else {
        let artist = params.artist.ok_or_else(|| {
            McpError::invalid_params("artist is required when track_id is not provided", None)
        })?;
        let title = params.title.ok_or_else(|| {
            McpError::invalid_params("title is required when track_id is not provided", None)
        })?;
        (artist, title, params.album)
    };

    let norm_artist = crate::normalize::normalize_for_matching(&artist);
    let norm_title = crate::normalize::normalize_for_matching(&title);

    if !force_refresh {
        let store_conn = server.cache_store_conn()?;
        if let Some(cached) =
            store::get_enrichment(&store_conn, "discogs", &norm_artist, &norm_title)
                .map_err(|e| mcp_internal_error(format!("Cache read error: {e}")))?
        {
            let result = match &cached.response_json {
                Some(json_str) => serde_json::from_str::<serde_json::Value>(json_str)
                    .unwrap_or(serde_json::Value::Null),
                None => serde_json::Value::Null,
            };
            let result =
                lookup_output_with_cache_metadata(result, true, Some(cached.created_at.as_str()));
            let json = serde_json::to_string_pretty(&result)
                .map_err(|e| mcp_internal_error(format!("{e}")))?;
            return Ok(CallToolResult::success(vec![Content::text(json)]));
        }
    }

    let result = lookup_discogs_remote(server, &artist, &title, album.as_deref())
        .await
        .map_err(|e| match e.auth_remediation() {
            Some(remediation) => mcp_internal_error(auth_remediation_message(remediation)),
            None => mcp_internal_error(format!("Discogs error: {e}")),
        })?;

    let (match_quality, response_json) = match &result {
        Some(r) => {
            let quality = if r.fuzzy_match { "fuzzy" } else { "exact" };
            let json = serde_json::to_string(r).map_err(|e| mcp_internal_error(format!("{e}")))?;
            (Some(quality), Some(json))
        }
        None => (Some("none"), None),
    };
    {
        let store_conn = server.cache_store_conn()?;
        store::set_enrichment(
            &store_conn,
            "discogs",
            &norm_artist,
            &norm_title,
            match_quality,
            response_json.as_deref(),
        )
        .map_err(|e| mcp_internal_error(format!("Cache write error: {e}")))?;
    }

    let output = lookup_output_with_cache_metadata(
        serde_json::to_value(&result).map_err(|e| mcp_internal_error(format!("{e}")))?,
        false,
        None,
    );
    let json =
        serde_json::to_string_pretty(&output).map_err(|e| mcp_internal_error(format!("{e}")))?;
    Ok(CallToolResult::success(vec![Content::text(json)]))
}

pub(super) async fn handle_lookup_beatport(
    server: &ReklawdboxServer,
    params: LookupBeatportParams,
) -> Result<CallToolResult, McpError> {
    let force_refresh = params.force_refresh.unwrap_or(false);

    // Resolve artist/title: from track_id or explicit params
    let (artist, title) = if let Some(ref track_id) = params.track_id {
        let conn = server.rekordbox_conn()?;
        let track = db::get_track(&conn, track_id)
            .map_err(|e| mcp_internal_error(format!("DB error: {e}")))?
            .ok_or_else(|| {
                McpError::invalid_params(format!("Track '{track_id}' not found"), None)
            })?;
        (
            params.artist.unwrap_or(track.artist),
            params.title.unwrap_or(track.title),
        )
    } else {
        let artist = params.artist.ok_or_else(|| {
            McpError::invalid_params("artist is required when track_id is not provided", None)
        })?;
        let title = params.title.ok_or_else(|| {
            McpError::invalid_params("title is required when track_id is not provided", None)
        })?;
        (artist, title)
    };

    let norm_artist = crate::normalize::normalize_for_matching(&artist);
    let norm_title = crate::normalize::normalize_for_matching(&title);

    if !force_refresh {
        let store_conn = server.cache_store_conn()?;
        if let Some(cached) =
            store::get_enrichment(&store_conn, "beatport", &norm_artist, &norm_title)
                .map_err(|e| mcp_internal_error(format!("Cache read error: {e}")))?
        {
            let result = match &cached.response_json {
                Some(json_str) => serde_json::from_str::<serde_json::Value>(json_str)
                    .unwrap_or(serde_json::Value::Null),
                None => serde_json::Value::Null,
            };
            let result =
                lookup_output_with_cache_metadata(result, true, Some(cached.created_at.as_str()));
            let json = serde_json::to_string_pretty(&result)
                .map_err(|e| mcp_internal_error(format!("{e}")))?;
            return Ok(CallToolResult::success(vec![Content::text(json)]));
        }
    }

    let result = lookup_beatport_remote(server, &artist, &title)
        .await
        .map_err(|e| mcp_internal_error(format!("Beatport error: {e}")))?;

    let (match_quality, response_json) = match &result {
        Some(r) => {
            let json = serde_json::to_string(r).map_err(|e| mcp_internal_error(format!("{e}")))?;
            (Some("exact"), Some(json))
        }
        None => (Some("none"), None),
    };
    {
        let store_conn = server.cache_store_conn()?;
        store::set_enrichment(
            &store_conn,
            "beatport",
            &norm_artist,
            &norm_title,
            match_quality,
            response_json.as_deref(),
        )
        .map_err(|e| mcp_internal_error(format!("Cache write error: {e}")))?;
    }

    let output = lookup_output_with_cache_metadata(
        serde_json::to_value(&result).map_err(|e| mcp_internal_error(format!("{e}")))?,
        false,
        None,
    );
    let json =
        serde_json::to_string_pretty(&output).map_err(|e| mcp_internal_error(format!("{e}")))?;
    Ok(CallToolResult::success(vec![Content::text(json)]))
}

// ---------------------------------------------------------------------------
// Enrichment cache writer message
// ---------------------------------------------------------------------------

enum EnrichCacheWriteMsg {
    Enrichment {
        provider: String,
        norm_artist: String,
        norm_title: String,
        match_quality: Option<String>,
        response_json: Option<String>,
    },
}

// ---------------------------------------------------------------------------
// Per-track enrichment result
// ---------------------------------------------------------------------------

struct EnrichTrackResult {
    processed: usize,
    cached: usize,
    skipped: usize,
    failures: Vec<serde_json::Value>,
    /// Set when a Discogs auth error is encountered.
    discogs_auth_error: Option<String>,
}

// ---------------------------------------------------------------------------
// Per-track enrichment function
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
async fn enrich_single_track(
    server: ReklawdboxServer,
    track_id: String,
    artist: String,
    title: String,
    album: String,
    norm_artist: String,
    norm_title: String,
    providers: Vec<crate::types::Provider>,
    skip_cached: bool,
    force_refresh: bool,
    store_path: String,
    cache_tx: tokio::sync::mpsc::Sender<EnrichCacheWriteMsg>,
    beatport_sem: std::sync::Arc<tokio::sync::Semaphore>,
    discogs_auth_failed: std::sync::Arc<tokio::sync::watch::Receiver<bool>>,
    auth_fail_tx: std::sync::Arc<tokio::sync::watch::Sender<bool>>,
) -> EnrichTrackResult {
    let mut result = EnrichTrackResult {
        processed: 0,
        cached: 0,
        skipped: 0,
        failures: Vec::new(),
        discogs_auth_error: None,
    };

    // Open read-only cache connection for cache checks
    let cache_conn = if skip_cached && !force_refresh {
        match store::open_read_only(&store_path) {
            Ok(c) => Some(c),
            Err(e) => {
                tracing::warn!("enrich_single_track: failed to open read-only store: {e}");
                None
            }
        }
    } else {
        None
    };

    // Determine which providers need work vs are cached
    let want_discogs = providers.contains(&crate::types::Provider::Discogs);
    let want_beatport = providers.contains(&crate::types::Provider::Beatport);

    let mut discogs_cached = false;
    let mut beatport_cached = false;

    if let Some(ref conn) = cache_conn {
        if want_discogs
            && let Ok(Some(_)) =
                store::get_enrichment(conn, "discogs", &norm_artist, &norm_title)
        {
            result.cached += 1;
            discogs_cached = true;
        }
        if want_beatport
            && let Ok(Some(_)) =
                store::get_enrichment(conn, "beatport", &norm_artist, &norm_title)
        {
            result.cached += 1;
            beatport_cached = true;
        }
    }

    // Drop the read connection before doing network I/O
    drop(cache_conn);

    let need_discogs = want_discogs && !discogs_cached;
    let need_beatport = want_beatport && !beatport_cached;

    // Build futures for each provider

    let discogs_fut = {
        let server = server.clone();
        let artist = artist.clone();
        let title = title.clone();
        let album = album.clone();
        let norm_artist = norm_artist.clone();
        let norm_title = norm_title.clone();
        let track_id = track_id.clone();
        let cache_tx = cache_tx.clone();
        let discogs_auth_failed = discogs_auth_failed.clone();
        let auth_fail_tx = auth_fail_tx.clone();
        async move {
            if !need_discogs {
                return (0usize, 0usize, Vec::new(), None);
            }

            // Check if auth already failed globally
            if *discogs_auth_failed.borrow() {
                return (
                    0,
                    0,
                    vec![serde_json::json!({
                        "track_id": &track_id,
                        "artist": &artist,
                        "title": &title,
                        "provider": "discogs",
                        "error": "Discogs auth failed (batch-wide)",
                    })],
                    None,
                );
            }

            let album_ref = if album.is_empty() {
                None
            } else {
                Some(album.as_str())
            };

            match lookup_discogs_remote(&server, &artist, &title, album_ref).await {
                Ok(Some(r)) => {
                    let quality = if r.fuzzy_match {
                        "fuzzy".to_string()
                    } else {
                        "exact".to_string()
                    };
                    let json_str = match serde_json::to_string(&r) {
                        Ok(j) => j,
                        Err(e) => {
                            return (
                                0,
                                0,
                                vec![serde_json::json!({
                                    "track_id": &track_id,
                                    "artist": &artist,
                                    "title": &title,
                                    "provider": "discogs",
                                    "error": format!("Serialize error: {e}"),
                                })],
                                None,
                            );
                        }
                    };
                    let _ = cache_tx
                        .send(EnrichCacheWriteMsg::Enrichment {
                            provider: "discogs".to_string(),
                            norm_artist,
                            norm_title,
                            match_quality: Some(quality),
                            response_json: Some(json_str),
                        })
                        .await;
                    (1, 0, Vec::new(), None)
                }
                Ok(None) => {
                    let _ = cache_tx
                        .send(EnrichCacheWriteMsg::Enrichment {
                            provider: "discogs".to_string(),
                            norm_artist,
                            norm_title,
                            match_quality: Some("none".to_string()),
                            response_json: None,
                        })
                        .await;
                    (0, 1, Vec::new(), None)
                }
                Err(e) => {
                    if let Some(remediation) = e.auth_remediation() {
                        let msg = auth_remediation_message(remediation);
                        // Broadcast auth failure to other tasks
                        let _ = auth_fail_tx.send(true);
                        (
                            0,
                            0,
                            vec![serde_json::json!({
                                "track_id": &track_id,
                                "artist": &artist,
                                "title": &title,
                                "provider": "discogs",
                                "error": &msg,
                            })],
                            Some(msg),
                        )
                    } else {
                        // Cache non-auth errors
                        let _ = cache_tx
                            .send(EnrichCacheWriteMsg::Enrichment {
                                provider: "discogs".to_string(),
                                norm_artist,
                                norm_title,
                                match_quality: Some("error".to_string()),
                                response_json: None,
                            })
                            .await;
                        (
                            0,
                            0,
                            vec![serde_json::json!({
                                "track_id": &track_id,
                                "artist": &artist,
                                "title": &title,
                                "provider": "discogs",
                                "error": e.to_string(),
                            })],
                            None,
                        )
                    }
                }
            }
        }
    };

    let beatport_fut = {
        let server = server.clone();
        let artist = artist.clone();
        let title = title.clone();
        let norm_artist = norm_artist.clone();
        let norm_title = norm_title.clone();
        let track_id = track_id.clone();
        let cache_tx = cache_tx.clone();
        let beatport_sem = beatport_sem.clone();
        async move {
            if !need_beatport {
                return (0usize, 0usize, Vec::new());
            }

            // Acquire Beatport semaphore to limit concurrent scraping
            let _permit = match beatport_sem.acquire().await {
                Ok(p) => p,
                Err(_) => {
                    return (
                        0,
                        0,
                        vec![serde_json::json!({
                            "track_id": &track_id,
                            "artist": &artist,
                            "title": &title,
                            "provider": "beatport",
                            "error": "Beatport semaphore closed",
                        })],
                    );
                }
            };

            match beatport::lookup(&server.state.http, &artist, &title).await {
                Ok(Some(r)) => {
                    let json_str = match serde_json::to_string(&r) {
                        Ok(j) => j,
                        Err(e) => {
                            return (
                                0,
                                0,
                                vec![serde_json::json!({
                                    "track_id": &track_id,
                                    "artist": &artist,
                                    "title": &title,
                                    "provider": "beatport",
                                    "error": format!("Serialize error: {e}"),
                                })],
                            );
                        }
                    };
                    let _ = cache_tx
                        .send(EnrichCacheWriteMsg::Enrichment {
                            provider: "beatport".to_string(),
                            norm_artist,
                            norm_title,
                            match_quality: Some("exact".to_string()),
                            response_json: Some(json_str),
                        })
                        .await;
                    (1, 0, Vec::new())
                }
                Ok(None) => {
                    let _ = cache_tx
                        .send(EnrichCacheWriteMsg::Enrichment {
                            provider: "beatport".to_string(),
                            norm_artist,
                            norm_title,
                            match_quality: Some("none".to_string()),
                            response_json: None,
                        })
                        .await;
                    (0, 1, Vec::new())
                }
                Err(e) => {
                    // Cache Beatport errors
                    let _ = cache_tx
                        .send(EnrichCacheWriteMsg::Enrichment {
                            provider: "beatport".to_string(),
                            norm_artist,
                            norm_title,
                            match_quality: Some("error".to_string()),
                            response_json: None,
                        })
                        .await;
                    (
                        0,
                        0,
                        vec![serde_json::json!({
                            "track_id": &track_id,
                            "artist": &artist,
                            "title": &title,
                            "provider": "beatport",
                            "error": e.to_string(),
                        })],
                    )
                }
            }
        }
    };

    // Run Discogs + Beatport in parallel when both are requested
    let (
        (discogs_processed, discogs_skipped, discogs_failures, discogs_auth_err),
        (beatport_processed, beatport_skipped, beatport_failures),
    ) = tokio::join!(discogs_fut, beatport_fut);

    result.processed += discogs_processed + beatport_processed;
    result.skipped += discogs_skipped + beatport_skipped;
    result.failures.extend(discogs_failures);
    result.failures.extend(beatport_failures);
    result.discogs_auth_error = discogs_auth_err;

    result
}

// ---------------------------------------------------------------------------
// Main batch enrichment handler
// ---------------------------------------------------------------------------

pub(super) async fn handle_enrich_tracks(
    server: &ReklawdboxServer,
    params: EnrichTracksParams,
) -> Result<CallToolResult, McpError> {
    let skip_cached = params.skip_cached.unwrap_or(true);
    let force_refresh = params.force_refresh.unwrap_or(false);
    let providers = params
        .providers
        .unwrap_or_else(|| vec![crate::types::Provider::Discogs]);

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
                default_max_tracks: Some(50),
                max_tracks_cap: Some(200),
                exclude_samplers: false,
            },
        )?
    };

    let total_tracks = tracks.len();
    let total = total_tracks.saturating_mul(providers.len());

    // Compute concurrency
    let concurrency = params
        .concurrency
        .map(|n| n.clamp(1, 8))
        .unwrap_or(4) as usize;

    let store_path = server.cache_store_path();

    // Ensure the DB exists and is migrated before spawning readers
    {
        let _conn = server.cache_store_conn()?;
    }

    // Spawn cache writer task
    let (cache_tx, mut cache_rx) =
        tokio::sync::mpsc::channel::<EnrichCacheWriteMsg>(concurrency * 4);
    let writer_store_path = store_path.clone();
    let writer_handle = tokio::task::spawn_blocking(move || {
        let conn = match store::open(&writer_store_path) {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("Enrich cache writer: failed to open store: {e}");
                return;
            }
        };
        while let Some(msg) = cache_rx.blocking_recv() {
            match msg {
                EnrichCacheWriteMsg::Enrichment {
                    provider,
                    norm_artist,
                    norm_title,
                    match_quality,
                    response_json,
                } => {
                    if let Err(e) = store::set_enrichment(
                        &conn,
                        &provider,
                        &norm_artist,
                        &norm_title,
                        match_quality.as_deref(),
                        response_json.as_deref(),
                    ) {
                        tracing::error!(
                            "Enrich cache writer: failed to write {provider} for {norm_artist}/{norm_title}: {e}"
                        );
                    }
                }
            }
        }
    });

    // Discogs auth failure broadcast
    let (auth_fail_tx, auth_fail_rx) = tokio::sync::watch::channel(false);
    let auth_fail_tx = std::sync::Arc::new(auth_fail_tx);
    let auth_fail_rx = std::sync::Arc::new(auth_fail_rx);

    // Semaphores
    let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(concurrency));
    let beatport_sem = std::sync::Arc::new(tokio::sync::Semaphore::new(2));

    // Spawn per-track tasks
    let mut handles = Vec::with_capacity(total_tracks);

    for track in &tracks {
        let permit = sem.clone().acquire_owned().await.map_err(|e| {
            mcp_internal_error(format!("Semaphore error: {e}"))
        })?;

        let server = server.clone();
        let track_id = track.id.clone();
        let artist = track.artist.clone();
        let title = track.title.clone();
        let album = track.album.clone();
        let norm_artist = crate::normalize::normalize_for_matching(&track.artist);
        let norm_title = crate::normalize::normalize_for_matching(&track.title);
        let providers = providers.clone();
        let store_path = store_path.clone();
        let cache_tx = cache_tx.clone();
        let beatport_sem = beatport_sem.clone();
        let auth_fail_rx = auth_fail_rx.clone();
        let auth_fail_tx = auth_fail_tx.clone();

        handles.push(tokio::spawn(async move {
            let result = enrich_single_track(
                server,
                track_id,
                artist,
                title,
                album,
                norm_artist,
                norm_title,
                providers,
                skip_cached,
                force_refresh,
                store_path,
                cache_tx,
                beatport_sem,
                auth_fail_rx,
                auth_fail_tx,
            )
            .await;
            drop(permit);
            result
        }));
    }

    // Collect results in order
    let mut progress = BatchProgress::new();

    for handle in handles {
        match handle.await {
            Ok(track_result) => {
                progress.processed += track_result.processed;
                progress.cached += track_result.cached;
                progress.skipped += track_result.skipped;
                progress.failures.extend(track_result.failures);
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
    let _ = writer_handle.await;

    let result = serde_json::json!({
        "summary": {
            "tracks_total": total_tracks,
            "total": total,
            "enriched": progress.processed,
            "cached": progress.cached,
            "skipped": progress.skipped,
            "failed": progress.failures.len(),
            "concurrency": concurrency,
        },
        "failures": progress.failures,
    });
    let json =
        serde_json::to_string_pretty(&result).map_err(|e| mcp_internal_error(format!("{e}")))?;
    Ok(CallToolResult::success(vec![Content::text(json)]))
}
