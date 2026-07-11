use rmcp::ErrorData as McpError;
use rmcp::model::CallToolResult;

use super::*;
use crate::bandcamp;
use crate::db;
use crate::musicbrainz;
use crate::store;

fn normalize_discogs_album_for_cache(album: Option<&str>) -> Option<String> {
    album
        .map(crate::normalize::normalize_for_matching)
        .filter(|album| !album.is_empty())
}

/// Resolve track identity from either track_id (DB lookup) or explicit artist/title.
fn resolve_lookup_identity(
    server: &ReklawdboxServer,
    track_id: Option<&str>,
    artist: Option<String>,
    title: Option<String>,
    album: Option<String>,
) -> Result<(String, String, Option<String>), McpError> {
    if let Some(track_id) = track_id {
        let conn = server.rekordbox_conn()?;
        let track = db::get_track(&conn, track_id)
            .map_err(db_error)?
            .ok_or_else(|| {
                McpError::invalid_params(format!("Track '{track_id}' not found"), None)
            })?;
        let album = album.or_else(|| (!track.album.is_empty()).then(|| track.album.clone()));
        Ok((
            artist.unwrap_or(track.artist),
            title.unwrap_or(track.title),
            album,
        ))
    } else {
        let artist = artist.ok_or_else(|| {
            McpError::invalid_params("artist is required when track_id is not provided", None)
        })?;
        let title = title.ok_or_else(|| {
            McpError::invalid_params("title is required when track_id is not provided", None)
        })?;
        Ok((artist, title, album))
    }
}

pub(super) async fn handle_lookup_discogs(
    server: &ReklawdboxServer,
    params: LookupDiscogsParams,
) -> Result<CallToolResult, McpError> {
    let force_refresh = params.force_refresh.unwrap_or(false);

    let (artist, title, album) = resolve_lookup_identity(
        server,
        params.track_id.as_deref(),
        params.artist,
        params.title,
        params.album,
    )?;

    let norm_artist = crate::normalize::normalize_for_matching(&artist);
    let norm_title = crate::normalize::normalize_for_matching(&title);
    let norm_album = normalize_discogs_album_for_cache(album.as_deref());

    if !force_refresh {
        let store_conn = server.cache_store_conn()?;
        if let Some(cached) = store::get_enrichment(
            &store_conn,
            "discogs",
            &norm_artist,
            &norm_title,
            norm_album.as_deref(),
            false,
        )
        .map_err(cache_error)?
        {
            let result = match &cached.response_json {
                Some(json_str) => serde_json::from_str::<serde_json::Value>(json_str)
                    .unwrap_or(serde_json::Value::Null),
                None => serde_json::Value::Null,
            };
            let result =
                lookup_output_with_cache_metadata(result, true, Some(cached.created_at.as_str()));
            return ok_json(&result);
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
            let json = serde_json::to_string(r).map_err(|e| mcp_internal_error(e.to_string()))?;
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
            norm_album.as_deref(),
            match_quality,
            response_json.as_deref(),
        )
        .map_err(cache_error)?;
    }

    let output = lookup_output_with_cache_metadata(
        serde_json::to_value(&result).map_err(|e| mcp_internal_error(e.to_string()))?,
        false,
        None,
    );
    ok_json(&output)
}

pub(super) async fn handle_lookup_beatport(
    server: &ReklawdboxServer,
    params: LookupBeatportParams,
) -> Result<CallToolResult, McpError> {
    let force_refresh = params.force_refresh.unwrap_or(false);

    let (artist, title, _) = resolve_lookup_identity(
        server,
        params.track_id.as_deref(),
        params.artist,
        params.title,
        None,
    )?;

    let norm_artist = crate::normalize::normalize_for_matching(&artist);
    let norm_title = crate::normalize::normalize_for_matching(&title);

    if !force_refresh {
        let store_conn = server.cache_store_conn()?;
        if let Some(cached) = store::get_enrichment(
            &store_conn,
            "beatport",
            &norm_artist,
            &norm_title,
            None,
            false,
        )
        .map_err(cache_error)?
        {
            let result = match &cached.response_json {
                Some(json_str) => serde_json::from_str::<serde_json::Value>(json_str)
                    .unwrap_or(serde_json::Value::Null),
                None => serde_json::Value::Null,
            };
            let result =
                lookup_output_with_cache_metadata(result, true, Some(cached.created_at.as_str()));
            return ok_json(&result);
        }
    }

    let result = lookup_beatport_remote(server, &artist, &title)
        .await
        .map_err(|e| mcp_internal_error(format!("Beatport error: {e}")))?;

    let (match_quality, response_json) = match &result {
        Some(r) => {
            let quality = if r.fuzzy_match { "fuzzy" } else { "exact" };
            let json = serde_json::to_string(r).map_err(|e| mcp_internal_error(e.to_string()))?;
            (Some(quality), Some(json))
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
            None,
            match_quality,
            response_json.as_deref(),
        )
        .map_err(cache_error)?;
    }

    let output = lookup_output_with_cache_metadata(
        serde_json::to_value(&result).map_err(|e| mcp_internal_error(e.to_string()))?,
        false,
        None,
    );
    ok_json(&output)
}

pub(super) async fn handle_lookup_musicbrainz(
    server: &ReklawdboxServer,
    params: LookupMusicBrainzParams,
) -> Result<CallToolResult, McpError> {
    let force_refresh = params.force_refresh.unwrap_or(false);

    let (artist, title, _) = resolve_lookup_identity(
        server,
        params.track_id.as_deref(),
        params.artist,
        params.title,
        None,
    )?;

    let norm_artist = crate::normalize::normalize_for_matching(&artist);
    let norm_title = crate::normalize::normalize_for_matching(&title);

    if !force_refresh {
        let store_conn = server.cache_store_conn()?;
        if let Some(cached) = store::get_enrichment(
            &store_conn,
            "musicbrainz",
            &norm_artist,
            &norm_title,
            None,
            false,
        )
        .map_err(cache_error)?
        {
            let result = match &cached.response_json {
                Some(json_str) => serde_json::from_str::<serde_json::Value>(json_str)
                    .unwrap_or(serde_json::Value::Null),
                None => serde_json::Value::Null,
            };
            let result =
                lookup_output_with_cache_metadata(result, true, Some(cached.created_at.as_str()));
            return ok_json(&result);
        }
    }

    let result = lookup_musicbrainz_remote(server, &artist, &title)
        .await
        .map_err(|e| mcp_internal_error(format!("MusicBrainz error: {e}")))?;

    let (match_quality, response_json) = match &result {
        Some(r) => {
            let quality = if r.score >= 100 { "exact" } else { "fuzzy" };
            let json = serde_json::to_string(r).map_err(|e| mcp_internal_error(e.to_string()))?;
            (Some(quality), Some(json))
        }
        None => (Some("none"), None),
    };
    {
        let store_conn = server.cache_store_conn()?;
        store::set_enrichment(
            &store_conn,
            "musicbrainz",
            &norm_artist,
            &norm_title,
            None,
            match_quality,
            response_json.as_deref(),
        )
        .map_err(cache_error)?;
    }

    let output = lookup_output_with_cache_metadata(
        serde_json::to_value(&result).map_err(|e| mcp_internal_error(e.to_string()))?,
        false,
        None,
    );
    ok_json(&output)
}

pub(super) async fn handle_lookup_bandcamp(
    server: &ReklawdboxServer,
    params: LookupBandcampParams,
) -> Result<CallToolResult, McpError> {
    let force_refresh = params.force_refresh.unwrap_or(false);
    let url = params.url;

    let (artist, title, _) = resolve_lookup_identity(
        server,
        params.track_id.as_deref(),
        params.artist,
        params.title,
        None,
    )?;

    let norm_artist = crate::normalize::normalize_for_matching(&artist);
    let norm_title = crate::normalize::normalize_for_matching(&title);

    if !force_refresh && url.is_none() {
        let store_conn = server.cache_store_conn()?;
        if let Some(cached) = store::get_enrichment(
            &store_conn,
            "bandcamp",
            &norm_artist,
            &norm_title,
            None,
            false,
        )
        .map_err(cache_error)?
        {
            let result = match &cached.response_json {
                Some(json_str) => serde_json::from_str::<serde_json::Value>(json_str)
                    .unwrap_or(serde_json::Value::Null),
                None => serde_json::Value::Null,
            };
            let result =
                lookup_output_with_cache_metadata(result, true, Some(cached.created_at.as_str()));
            return ok_json(&result);
        }
    }

    let result = if let Some(url) = url {
        bandcamp::lookup_url(&server.state.http, &url, &artist, &title)
            .await
            .map_err(|e| mcp_internal_error(format!("Bandcamp error: {e}")))?
    } else {
        lookup_bandcamp_remote(server, &artist, &title)
            .await
            .map_err(|e| mcp_internal_error(format!("Bandcamp error: {e}")))?
    };

    let (match_quality, response_json) = match &result {
        Some(r) => {
            let quality = if r.score == 100 { "exact" } else { "fuzzy" };
            let json = serde_json::to_string(r).map_err(|e| mcp_internal_error(e.to_string()))?;
            (Some(quality), Some(json))
        }
        None => (Some("none"), None),
    };
    {
        let store_conn = server.cache_store_conn()?;
        store::set_enrichment(
            &store_conn,
            "bandcamp",
            &norm_artist,
            &norm_title,
            None,
            match_quality,
            response_json.as_deref(),
        )
        .map_err(cache_error)?;
    }

    let output = lookup_output_with_cache_metadata(
        serde_json::to_value(&result).map_err(|e| mcp_internal_error(e.to_string()))?,
        false,
        None,
    );
    ok_json(&output)
}

pub(super) async fn lookup_musicbrainz_remote(
    server: &ReklawdboxServer,
    artist: &str,
    title: &str,
) -> Result<Option<musicbrainz::MusicBrainzResult>, String> {
    musicbrainz::lookup(&server.state.http, artist, title)
        .await
        .map_err(|e| e.to_string())
}

enum EnrichCacheWriteMsg {
    Enrichment {
        provider: String,
        norm_artist: String,
        norm_title: String,
        norm_album: Option<String>,
        match_quality: Option<String>,
        response_json: Option<String>,
        acknowledgement: tokio::sync::oneshot::Sender<Result<(), String>>,
    },
}

struct EnrichCacheWrite {
    provider: String,
    norm_artist: String,
    norm_title: String,
    norm_album: Option<String>,
    match_quality: Option<String>,
    response_json: Option<String>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct EnrichCacheWriterReport {
    attempted: usize,
    succeeded: usize,
    failed: usize,
    dropped_ack_receivers: usize,
}

async fn write_enrichment_cache(
    cache_tx: &tokio::sync::mpsc::Sender<EnrichCacheWriteMsg>,
    write: EnrichCacheWrite,
) -> Result<(), String> {
    let (acknowledgement, acknowledged) = tokio::sync::oneshot::channel();
    cache_tx
        .send(EnrichCacheWriteMsg::Enrichment {
            provider: write.provider,
            norm_artist: write.norm_artist,
            norm_title: write.norm_title,
            norm_album: write.norm_album,
            match_quality: write.match_quality,
            response_json: write.response_json,
            acknowledgement,
        })
        .await
        .map_err(|_| "cache write queue closed".to_string())?;

    acknowledged
        .await
        .map_err(|_| "cache writer acknowledgement canceled".to_string())?
}

fn run_enrich_cache_writer(
    store_path: &str,
    mut cache_rx: tokio::sync::mpsc::Receiver<EnrichCacheWriteMsg>,
) -> EnrichCacheWriterReport {
    let connection =
        store::open(store_path).map_err(|error| format!("cache writer open failed: {error}"));
    if let Err(error) = &connection {
        tracing::error!("Enrich cache writer: {error}");
    }

    let mut report = EnrichCacheWriterReport::default();
    while let Some(msg) = cache_rx.blocking_recv() {
        match msg {
            EnrichCacheWriteMsg::Enrichment {
                provider,
                norm_artist,
                norm_title,
                norm_album,
                match_quality,
                response_json,
                acknowledgement,
            } => {
                report.attempted += 1;
                let result = match &connection {
                    Ok(conn) => store::set_enrichment(
                        conn,
                        &provider,
                        &norm_artist,
                        &norm_title,
                        norm_album.as_deref(),
                        match_quality.as_deref(),
                        response_json.as_deref(),
                    )
                    .map_err(|error| format!("cache write failed: {error}")),
                    Err(error) => Err(error.clone()),
                };

                if result.is_ok() {
                    report.succeeded += 1;
                } else {
                    report.failed += 1;
                }
                if acknowledgement.send(result).is_err() {
                    report.dropped_ack_receivers += 1;
                }
            }
        }
    }

    debug_assert_eq!(report.attempted, report.succeeded + report.failed);
    report
}

fn cache_write_failure(
    track_id: &str,
    artist: &str,
    title: &str,
    provider: &str,
    error: String,
) -> serde_json::Value {
    serde_json::json!({
        "track_id": track_id,
        "artist": artist,
        "title": title,
        "provider": provider,
        "error": error,
    })
}

/// Shared enrichment future for Beatport and Bandcamp: acquire semaphore, run
/// lookup, determine match quality, send cache write, return (processed, skipped,
/// failures).  Discogs has distinct auth/broadcast logic and stays separate.
#[allow(clippy::too_many_arguments)]
async fn provider_enrich_fut<T, E>(
    provider: &str,
    need: bool,
    track_id: &str,
    artist: &str,
    title: &str,
    norm_artist: String,
    norm_title: String,
    semaphore: std::sync::Arc<tokio::sync::Semaphore>,
    cache_tx: &tokio::sync::mpsc::Sender<EnrichCacheWriteMsg>,
    lookup_fut: impl std::future::Future<Output = Result<Option<T>, E>>,
    quality_fn: impl FnOnce(&T) -> &str,
) -> (usize, usize, Vec<serde_json::Value>)
where
    T: serde::Serialize,
    E: std::fmt::Display,
{
    if !need {
        return (0, 0, Vec::new());
    }

    let _permit = match semaphore.acquire().await {
        Ok(p) => p,
        Err(_) => {
            return (
                0,
                0,
                vec![serde_json::json!({
                    "track_id": track_id,
                    "artist": artist,
                    "title": title,
                    "provider": provider,
                    "error": format!("{provider} semaphore closed"),
                })],
            );
        }
    };

    match lookup_fut.await {
        Ok(Some(r)) => {
            let quality = quality_fn(&r);
            let json_str = match serde_json::to_string(&r) {
                Ok(j) => j,
                Err(e) => {
                    return (
                        0,
                        0,
                        vec![serde_json::json!({
                            "track_id": track_id,
                            "artist": artist,
                            "title": title,
                            "provider": provider,
                            "error": format!("Serialize error: {e}"),
                        })],
                    );
                }
            };
            match write_enrichment_cache(
                cache_tx,
                EnrichCacheWrite {
                    provider: provider.to_string(),
                    norm_artist,
                    norm_title,
                    norm_album: None,
                    match_quality: Some(quality.to_string()),
                    response_json: Some(json_str),
                },
            )
            .await
            {
                Ok(()) => (1, 0, Vec::new()),
                Err(error) => (
                    0,
                    0,
                    vec![cache_write_failure(
                        track_id, artist, title, provider, error,
                    )],
                ),
            }
        }
        Ok(None) => {
            match write_enrichment_cache(
                cache_tx,
                EnrichCacheWrite {
                    provider: provider.to_string(),
                    norm_artist,
                    norm_title,
                    norm_album: None,
                    match_quality: Some("none".to_string()),
                    response_json: None,
                },
            )
            .await
            {
                Ok(()) => (0, 1, Vec::new()),
                Err(error) => (
                    0,
                    0,
                    vec![cache_write_failure(
                        track_id, artist, title, provider, error,
                    )],
                ),
            }
        }
        Err(e) => (
            0,
            0,
            vec![serde_json::json!({
                "track_id": track_id,
                "artist": artist,
                "title": title,
                "provider": provider,
                "error": e.to_string(),
            })],
        ),
    }
}

struct EnrichTrackResult {
    processed: usize,
    cached: usize,
    skipped: usize,
    failures: Vec<serde_json::Value>,
    /// Set when a Discogs auth error is encountered.
    discogs_auth_error: Option<String>,
}

#[allow(clippy::too_many_arguments)]
async fn enrich_single_track(
    server: ReklawdboxServer,
    track_id: String,
    artist: String,
    title: String,
    album: String,
    norm_artist: String,
    norm_title: String,
    norm_album: Option<String>,
    providers: Vec<crate::types::Provider>,
    skip_cached: bool,
    force_refresh: bool,
    store_path: String,
    cache_tx: tokio::sync::mpsc::Sender<EnrichCacheWriteMsg>,
    beatport_sem: std::sync::Arc<tokio::sync::Semaphore>,
    bandcamp_sem: std::sync::Arc<tokio::sync::Semaphore>,
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

    let want_discogs = providers.contains(&crate::types::Provider::Discogs);
    let want_beatport = providers.contains(&crate::types::Provider::Beatport);
    let want_bandcamp = providers.contains(&crate::types::Provider::Bandcamp);

    let mut discogs_cached = false;
    let mut beatport_cached = false;
    let mut bandcamp_cached = false;

    if let Some(ref conn) = cache_conn {
        if want_discogs
            && let Ok(Some(_)) = store::get_enrichment(
                conn,
                "discogs",
                &norm_artist,
                &norm_title,
                norm_album.as_deref(),
                false,
            )
        {
            result.cached += 1;
            discogs_cached = true;
        }
        if want_beatport
            && let Ok(Some(_)) =
                store::get_enrichment(conn, "beatport", &norm_artist, &norm_title, None, false)
        {
            result.cached += 1;
            beatport_cached = true;
        }
        if want_bandcamp
            && let Ok(Some(_)) =
                store::get_enrichment(conn, "bandcamp", &norm_artist, &norm_title, None, false)
        {
            result.cached += 1;
            bandcamp_cached = true;
        }
    }

    // Drop the read connection before doing network I/O
    drop(cache_conn);

    let need_discogs = want_discogs && !discogs_cached;
    let need_beatport = want_beatport && !beatport_cached;
    let need_bandcamp = want_bandcamp && !bandcamp_cached;

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
                    match write_enrichment_cache(
                        &cache_tx,
                        EnrichCacheWrite {
                            provider: "discogs".to_string(),
                            norm_artist,
                            norm_title,
                            norm_album,
                            match_quality: Some(quality),
                            response_json: Some(json_str),
                        },
                    )
                    .await
                    {
                        Ok(()) => (1, 0, Vec::new(), None),
                        Err(error) => (
                            0,
                            0,
                            vec![cache_write_failure(
                                &track_id, &artist, &title, "discogs", error,
                            )],
                            None,
                        ),
                    }
                }
                Ok(None) => {
                    match write_enrichment_cache(
                        &cache_tx,
                        EnrichCacheWrite {
                            provider: "discogs".to_string(),
                            norm_artist,
                            norm_title,
                            norm_album,
                            match_quality: Some("none".to_string()),
                            response_json: None,
                        },
                    )
                    .await
                    {
                        Ok(()) => (0, 1, Vec::new(), None),
                        Err(error) => (
                            0,
                            0,
                            vec![cache_write_failure(
                                &track_id, &artist, &title, "discogs", error,
                            )],
                            None,
                        ),
                    }
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
            provider_enrich_fut(
                "beatport",
                need_beatport,
                &track_id,
                &artist,
                &title,
                norm_artist,
                norm_title,
                beatport_sem,
                &cache_tx,
                lookup_beatport_remote(&server, &artist, &title),
                |r| if r.fuzzy_match { "fuzzy" } else { "exact" },
            )
            .await
        }
    };

    let bandcamp_fut = {
        let server = server.clone();
        let artist = artist.clone();
        let title = title.clone();
        let norm_artist = norm_artist.clone();
        let norm_title = norm_title.clone();
        let track_id = track_id.clone();
        let cache_tx = cache_tx.clone();
        let bandcamp_sem = bandcamp_sem.clone();
        async move {
            provider_enrich_fut(
                "bandcamp",
                need_bandcamp,
                &track_id,
                &artist,
                &title,
                norm_artist,
                norm_title,
                bandcamp_sem,
                &cache_tx,
                bandcamp::lookup(&server.state.http, &artist, &title),
                |r| if r.score == 100 { "exact" } else { "fuzzy" },
            )
            .await
        }
    };

    let (
        (discogs_processed, discogs_skipped, discogs_failures, discogs_auth_err),
        (beatport_processed, beatport_skipped, beatport_failures),
        (bandcamp_processed, bandcamp_skipped, bandcamp_failures),
    ) = tokio::join!(discogs_fut, beatport_fut, bandcamp_fut);

    result.processed += discogs_processed + beatport_processed + bandcamp_processed;
    result.skipped += discogs_skipped + beatport_skipped + bandcamp_skipped;
    result.failures.extend(discogs_failures);
    result.failures.extend(beatport_failures);
    result.failures.extend(bandcamp_failures);
    result.discogs_auth_error = discogs_auth_err;

    result
}

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

    let concurrency = params.concurrency.map_or(4, |n| n.clamp(1, 8)) as usize;

    let store_path = server.cache_store_path();

    // Ensure the DB exists and is migrated before spawning readers
    {
        let _conn = server.cache_store_conn()?;
    }

    let (cache_tx, cache_rx) = tokio::sync::mpsc::channel::<EnrichCacheWriteMsg>(concurrency * 4);
    let writer_store_path = store_path.clone();
    let writer_handle =
        tokio::task::spawn_blocking(move || run_enrich_cache_writer(&writer_store_path, cache_rx));

    let (auth_fail_tx, auth_fail_rx) = tokio::sync::watch::channel(false);
    let auth_fail_tx = std::sync::Arc::new(auth_fail_tx);
    let auth_fail_rx = std::sync::Arc::new(auth_fail_rx);

    let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(concurrency));
    let beatport_sem = std::sync::Arc::new(tokio::sync::Semaphore::new(2));
    let bandcamp_sem = std::sync::Arc::new(tokio::sync::Semaphore::new(2));

    let mut handles = Vec::with_capacity(total_tracks);

    for track in &tracks {
        let permit = sem
            .clone()
            .acquire_owned()
            .await
            .map_err(|e| mcp_internal_error(format!("Semaphore error: {e}")))?;

        let server = server.clone();
        let track_id = track.id.clone();
        let artist = track.artist.clone();
        let title = track.title.clone();
        let album = track.album.clone();
        let norm_artist = crate::normalize::normalize_for_matching(&track.artist);
        let norm_title = crate::normalize::normalize_for_matching(&track.title);
        let norm_album = normalize_discogs_album_for_cache(Some(&track.album));
        let providers = providers.clone();
        let store_path = store_path.clone();
        let cache_tx = cache_tx.clone();
        let beatport_sem = beatport_sem.clone();
        let bandcamp_sem = bandcamp_sem.clone();
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
                norm_album,
                providers,
                skip_cached,
                force_refresh,
                store_path,
                cache_tx,
                beatport_sem,
                bandcamp_sem,
                auth_fail_rx,
                auth_fail_tx,
            )
            .await;
            drop(permit);
            result
        }));
    }

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

    drop(cache_tx);
    let cache_write_report = match writer_handle.await {
        Ok(report) => {
            debug_assert_eq!(report.attempted, report.succeeded + report.failed);
            report
        }
        Err(e) => {
            progress.failures.push(serde_json::json!({
                "provider": "cache_writer",
                "error": format!("Cache writer task failed: {e}"),
            }));
            EnrichCacheWriterReport::default()
        }
    };

    let result = serde_json::json!({
        "summary": {
            "tracks_total": total_tracks,
            "total": total,
            "enriched": progress.processed,
            "cached": progress.cached,
            "skipped": progress.skipped,
            "failed": progress.failures.len(),
            "concurrency": concurrency,
            "cache_writes": {
                "attempted": cache_write_report.attempted,
                "succeeded": cache_write_report.succeeded,
                "failed": cache_write_report.failed,
            },
        },
        "failures": progress.failures,
    });
    ok_json(&result)
}

#[cfg(test)]
mod cache_write_tests {
    use super::*;
    use std::future::Future;
    use std::time::Duration;

    const OUTER_TIMEOUT: Duration = Duration::from_secs(5);
    const STEP_TIMEOUT: Duration = Duration::from_secs(2);

    fn test_write(title: &str) -> EnrichCacheWrite {
        EnrichCacheWrite {
            provider: "beatport".to_string(),
            norm_artist: "test artist".to_string(),
            norm_title: title.to_string(),
            norm_album: None,
            match_quality: Some("none".to_string()),
            response_json: None,
        }
    }

    async fn bounded<T>(phase: &str, future: impl Future<Output = T>) -> Result<T, String> {
        tokio::time::timeout(STEP_TIMEOUT, future)
            .await
            .map_err(|_| format!("{phase} timed out"))
    }

    struct AckTaskGuard {
        sender: Option<tokio::sync::mpsc::Sender<EnrichCacheWriteMsg>>,
        writer: Option<tokio::task::JoinHandle<Result<(), String>>>,
    }

    impl AckTaskGuard {
        fn new(
            sender: tokio::sync::mpsc::Sender<EnrichCacheWriteMsg>,
            writer: tokio::task::JoinHandle<Result<(), String>>,
        ) -> Self {
            Self {
                sender: Some(sender),
                writer: Some(writer),
            }
        }

        fn sender(&self) -> &tokio::sync::mpsc::Sender<EnrichCacheWriteMsg> {
            self.sender
                .as_ref()
                .expect("ack test sender should remain until cleanup")
        }

        async fn finish(&mut self) -> Result<(), String> {
            self.sender.take();
            let mut writer = self
                .writer
                .take()
                .ok_or_else(|| "ack test writer handle is missing".to_string())?;
            match tokio::time::timeout(STEP_TIMEOUT, &mut writer).await {
                Ok(Ok(result)) => result,
                Ok(Err(error)) => Err(format!("ack test writer join failed: {error}")),
                Err(_) => {
                    writer.abort();
                    let _ = tokio::time::timeout(STEP_TIMEOUT, &mut writer).await;
                    Err("ack test writer join timed out".to_string())
                }
            }
        }
    }

    impl Drop for AckTaskGuard {
        fn drop(&mut self) {
            self.sender.take();
            if let Some(writer) = self.writer.take() {
                writer.abort();
            }
        }
    }

    async fn exercise_ack(
        acknowledgement: Option<Result<(), String>>,
    ) -> Result<Result<(), String>, String> {
        let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
        let writer = tokio::spawn(async move {
            let message = bounded("ack channel receive", receiver.recv())
                .await?
                .ok_or_else(|| "ack channel closed before a message arrived".to_string())?;
            match message {
                EnrichCacheWriteMsg::Enrichment {
                    acknowledgement: sender,
                    ..
                } => {
                    if let Some(result) = acknowledgement {
                        sender
                            .send(result)
                            .map_err(|_| "ack receiver dropped unexpectedly".to_string())?;
                    }
                }
            }
            Ok(())
        });
        let mut guard = AckTaskGuard::new(sender, writer);
        let result = bounded(
            "oneshot acknowledgement",
            write_enrichment_cache(guard.sender(), test_write("ack test")),
        )
        .await;
        let cleanup = guard.finish().await;
        cleanup?;
        result
    }

    #[tokio::test]
    async fn enrich_cache_ack_success() {
        let result = tokio::time::timeout(OUTER_TIMEOUT, exercise_ack(Some(Ok(()))))
            .await
            .expect("ack success scenario should finish within five seconds")
            .expect("ack success harness should finish cleanly");
        assert_eq!(result, Ok(()));
    }

    #[tokio::test]
    async fn enrich_cache_ack_explicit_writer_error() {
        let result = tokio::time::timeout(
            OUTER_TIMEOUT,
            exercise_ack(Some(Err("cache write failed: sentinel".to_string()))),
        )
        .await
        .expect("ack error scenario should finish within five seconds")
        .expect("ack error harness should finish cleanly");
        assert_eq!(result, Err("cache write failed: sentinel".to_string()));
    }

    #[tokio::test]
    async fn enrich_cache_ack_closed_queue() {
        let result = tokio::time::timeout(OUTER_TIMEOUT, async {
            let (sender, receiver) = tokio::sync::mpsc::channel(1);
            drop(receiver);
            bounded(
                "closed queue send",
                write_enrichment_cache(&sender, test_write("closed queue")),
            )
            .await
        })
        .await
        .expect("closed queue scenario should finish within five seconds")
        .expect("closed queue bounded wait should finish");
        assert_eq!(result, Err("cache write queue closed".to_string()));
    }

    #[tokio::test]
    async fn enrich_cache_ack_canceled_sender() {
        let result = tokio::time::timeout(OUTER_TIMEOUT, exercise_ack(None))
            .await
            .expect("ack cancellation scenario should finish within five seconds")
            .expect("ack cancellation harness should finish cleanly");
        assert_eq!(
            result,
            Err("cache writer acknowledgement canceled".to_string())
        );
    }

    struct WriterHarness {
        sender: Option<tokio::sync::mpsc::Sender<EnrichCacheWriteMsg>>,
        writer: Option<tokio::task::JoinHandle<EnrichCacheWriterReport>>,
    }

    impl WriterHarness {
        fn new(store_path: String) -> Self {
            let (sender, receiver) = tokio::sync::mpsc::channel(2);
            let writer =
                tokio::task::spawn_blocking(move || run_enrich_cache_writer(&store_path, receiver));
            Self {
                sender: Some(sender),
                writer: Some(writer),
            }
        }

        fn sender(&self) -> &tokio::sync::mpsc::Sender<EnrichCacheWriteMsg> {
            self.sender
                .as_ref()
                .expect("writer test sender should remain until cleanup")
        }

        async fn write(&self, write: EnrichCacheWrite) -> Result<(), String> {
            bounded(
                "writer acknowledgement",
                write_enrichment_cache(self.sender(), write),
            )
            .await?
        }

        async fn send_with_dropped_ack(&self, write: EnrichCacheWrite) -> Result<(), String> {
            let (acknowledgement, acknowledged) = tokio::sync::oneshot::channel();
            drop(acknowledged);
            bounded(
                "dropped-ack channel send",
                self.sender().send(EnrichCacheWriteMsg::Enrichment {
                    provider: write.provider,
                    norm_artist: write.norm_artist,
                    norm_title: write.norm_title,
                    norm_album: write.norm_album,
                    match_quality: write.match_quality,
                    response_json: write.response_json,
                    acknowledgement,
                }),
            )
            .await?
            .map_err(|_| "writer queue closed during dropped-ack test".to_string())
        }

        async fn finish(&mut self) -> Result<EnrichCacheWriterReport, String> {
            self.sender.take();
            let mut writer = self
                .writer
                .take()
                .ok_or_else(|| "writer test handle is missing".to_string())?;
            match tokio::time::timeout(STEP_TIMEOUT, &mut writer).await {
                Ok(Ok(report)) => Ok(report),
                Ok(Err(error)) => Err(format!("writer test join failed: {error}")),
                Err(_) => {
                    writer.abort();
                    let _ = tokio::time::timeout(STEP_TIMEOUT, &mut writer).await;
                    Err("writer test join timed out".to_string())
                }
            }
        }
    }

    impl Drop for WriterHarness {
        fn drop(&mut self) {
            self.sender.take();
            if let Some(writer) = self.writer.take() {
                writer.abort();
            }
        }
    }

    #[tokio::test]
    async fn enrich_cache_writer_persists_and_reports_success() {
        let (result, report) = tokio::time::timeout(OUTER_TIMEOUT, async {
            let store_dir = tempfile::tempdir().expect("writer store dir should create");
            let store_path = store_dir.path().join("internal.sqlite3");
            let mut harness = WriterHarness::new(store_path.to_string_lossy().to_string());
            let result = harness.write(test_write("writer success")).await;
            let report = harness.finish().await?;
            Ok::<_, String>((result, report))
        })
        .await
        .expect("writer success scenario should finish within five seconds")
        .expect("writer success harness should finish cleanly");

        assert_eq!(result, Ok(()));
        assert_eq!(
            report,
            EnrichCacheWriterReport {
                attempted: 1,
                succeeded: 1,
                failed: 0,
                dropped_ack_receivers: 0,
            }
        );
    }

    #[tokio::test]
    async fn enrich_cache_writer_continues_after_selective_failure() {
        let (rejected, accepted, report) = tokio::time::timeout(OUTER_TIMEOUT, async {
            let store_dir = tempfile::tempdir().expect("writer store dir should create");
            let store_path = store_dir.path().join("internal.sqlite3");
            let conn = store::open(&store_path.to_string_lossy())
                .expect("writer test store should initialize");
            conn.execute_batch(
                "CREATE TRIGGER fail_selected_enrichment
                 BEFORE INSERT ON enrichment_cache
                 WHEN NEW.query_title = 'reject this'
                 BEGIN
                     SELECT RAISE(FAIL, 'selected cache write failure');
                 END;",
            )
            .expect("selective failure trigger should install");
            drop(conn);

            let mut harness = WriterHarness::new(store_path.to_string_lossy().to_string());
            let rejected = harness.write(test_write("reject this")).await;
            let accepted = harness.write(test_write("accept this")).await;
            let report = harness.finish().await?;
            Ok::<_, String>((rejected, accepted, report))
        })
        .await
        .expect("selective writer scenario should finish within five seconds")
        .expect("selective writer harness should finish cleanly");

        assert!(
            rejected
                .expect_err("selected cache write should fail")
                .starts_with("cache write failed:")
        );
        assert_eq!(accepted, Ok(()));
        assert_eq!(report.attempted, 2);
        assert_eq!(report.succeeded, 1);
        assert_eq!(report.failed, 1);
    }

    #[tokio::test]
    async fn enrich_cache_writer_open_failure_drains_all_messages() {
        let (first, second, report) = tokio::time::timeout(OUTER_TIMEOUT, async {
            let store_dir = tempfile::tempdir().expect("writer store dir should create");
            let mut harness = WriterHarness::new(store_dir.path().to_string_lossy().to_string());
            let first = harness.write(test_write("open failure one")).await;
            let second = harness.write(test_write("open failure two")).await;
            let report = harness.finish().await?;
            Ok::<_, String>((first, second, report))
        })
        .await
        .expect("writer-open scenario should finish within five seconds")
        .expect("writer-open harness should finish cleanly");

        for result in [first, second] {
            assert!(
                result
                    .expect_err("writer-open failure should reject every message")
                    .starts_with("cache writer open failed:")
            );
        }
        assert_eq!(report.attempted, 2);
        assert_eq!(report.succeeded, 0);
        assert_eq!(report.failed, 2);
    }

    #[tokio::test]
    async fn enrich_cache_writer_records_dropped_ack_and_continues() {
        let (next, report) = tokio::time::timeout(OUTER_TIMEOUT, async {
            let store_dir = tempfile::tempdir().expect("writer store dir should create");
            let store_path = store_dir.path().join("internal.sqlite3");
            let mut harness = WriterHarness::new(store_path.to_string_lossy().to_string());
            let dropped = harness
                .send_with_dropped_ack(test_write("dropped ack"))
                .await;
            let next = harness.write(test_write("after dropped ack")).await;
            let report = harness.finish().await?;
            dropped?;
            Ok::<_, String>((next, report))
        })
        .await
        .expect("dropped-ack scenario should finish within five seconds")
        .expect("dropped-ack harness should finish cleanly");

        assert_eq!(next, Ok(()));
        assert_eq!(report.attempted, 2);
        assert_eq!(report.succeeded, 2);
        assert_eq!(report.failed, 0);
        assert_eq!(report.dropped_ack_receivers, 1);
    }
}
