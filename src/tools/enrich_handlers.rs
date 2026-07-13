use rmcp::ErrorData as McpError;
use rmcp::model::CallToolResult;

use super::*;
use crate::application::analysis::batch::CacheWriteRequest;
use crate::application::enrichment::hydrate::{
    EnrichmentCacheWrite, EnrichmentCacheWriterReport, HydrationWorkerCompletion,
    acknowledge_enrichment_cache_write, provider_stages, run_bounded_workers,
    run_enrichment_cache_writer,
};
use crate::application::enrichment::lookup::{
    CachedLookup, LookupCacheWrite, LookupPolicy, LookupWorkflowError, lookup_with_cache,
};
use crate::bandcamp;
use crate::db;
use crate::musicbrainz;
use crate::store;

fn normalize_discogs_album_for_cache(album: Option<&str>) -> Option<String> {
    album
        .map(crate::normalize::normalize_for_matching)
        .filter(|album| !album.is_empty())
}

fn read_lookup_cache(
    server: &ReklawdboxServer,
    provider: &str,
    norm_artist: &str,
    norm_title: &str,
    norm_album: Option<&str>,
) -> Result<Option<CachedLookup>, McpError> {
    let store_conn = server.cache_store_conn()?;
    store::get_enrichment(
        &store_conn,
        provider,
        norm_artist,
        norm_title,
        norm_album,
        false,
    )
    .map(|cached| {
        cached.map(|cached| CachedLookup {
            response_json: cached.response_json,
            created_at: cached.created_at,
        })
    })
    .map_err(cache_error)
}

fn write_lookup_cache(
    server: &ReklawdboxServer,
    provider: &str,
    norm_artist: &str,
    norm_title: &str,
    norm_album: Option<&str>,
    write: LookupCacheWrite,
) -> Result<(), McpError> {
    let store_conn = server.cache_store_conn()?;
    store::set_enrichment(
        &store_conn,
        provider,
        norm_artist,
        norm_title,
        norm_album,
        Some(&write.match_quality),
        write.response_json.as_deref(),
    )
    .map_err(cache_error)
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

    let result = lookup_with_cache(
        LookupPolicy {
            force_refresh,
            cache_read_enabled: true,
        },
        || {
            read_lookup_cache(
                server,
                "discogs",
                &norm_artist,
                &norm_title,
                norm_album.as_deref(),
            )
        },
        |write| {
            write_lookup_cache(
                server,
                "discogs",
                &norm_artist,
                &norm_title,
                norm_album.as_deref(),
                write,
            )
        },
        lookup_discogs_remote(server, &artist, &title, album.as_deref()),
        |result| {
            if result.fuzzy_match { "fuzzy" } else { "exact" }
        },
    )
    .await
    .map_err(|error| match error {
        LookupWorkflowError::Cache(error) => error,
        LookupWorkflowError::Lookup(error) => match error.auth_remediation() {
            Some(remediation) => mcp_internal_error(auth_remediation_message(remediation)),
            None => mcp_internal_error(format!("Discogs error: {error}")),
        },
        LookupWorkflowError::Serialize(error) => mcp_internal_error(error.to_string()),
    })?;

    ok_json(&result.into_output())
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

    let result = lookup_with_cache(
        LookupPolicy {
            force_refresh,
            cache_read_enabled: true,
        },
        || read_lookup_cache(server, "beatport", &norm_artist, &norm_title, None),
        |write| write_lookup_cache(server, "beatport", &norm_artist, &norm_title, None, write),
        lookup_beatport_remote(server, &artist, &title),
        |result| {
            if result.fuzzy_match { "fuzzy" } else { "exact" }
        },
    )
    .await
    .map_err(|error| match error {
        LookupWorkflowError::Cache(error) => error,
        LookupWorkflowError::Lookup(error) => {
            mcp_internal_error(format!("Beatport error: {error}"))
        }
        LookupWorkflowError::Serialize(error) => mcp_internal_error(error.to_string()),
    })?;

    ok_json(&result.into_output())
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

    let result = lookup_with_cache(
        LookupPolicy {
            force_refresh,
            cache_read_enabled: true,
        },
        || read_lookup_cache(server, "musicbrainz", &norm_artist, &norm_title, None),
        |write| {
            write_lookup_cache(
                server,
                "musicbrainz",
                &norm_artist,
                &norm_title,
                None,
                write,
            )
        },
        lookup_musicbrainz_remote(server, &artist, &title),
        |result| {
            if result.score >= 100 {
                "exact"
            } else {
                "fuzzy"
            }
        },
    )
    .await
    .map_err(|error| match error {
        LookupWorkflowError::Cache(error) => error,
        LookupWorkflowError::Lookup(error) => {
            mcp_internal_error(format!("MusicBrainz error: {error}"))
        }
        LookupWorkflowError::Serialize(error) => mcp_internal_error(error.to_string()),
    })?;

    ok_json(&result.into_output())
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

    let cache_read_enabled = url.is_none();
    let lookup = async {
        if let Some(url) = url {
            bandcamp::lookup_url(&server.state.http, &url, &artist, &title)
                .await
                .map_err(|error| error.to_string())
        } else {
            lookup_bandcamp_remote(server, &artist, &title).await
        }
    };
    let result = lookup_with_cache(
        LookupPolicy {
            force_refresh,
            cache_read_enabled,
        },
        || read_lookup_cache(server, "bandcamp", &norm_artist, &norm_title, None),
        |write| write_lookup_cache(server, "bandcamp", &norm_artist, &norm_title, None, write),
        lookup,
        |result| {
            if result.score == 100 {
                "exact"
            } else {
                "fuzzy"
            }
        },
    )
    .await
    .map_err(|error| match error {
        LookupWorkflowError::Cache(error) => error,
        LookupWorkflowError::Lookup(error) => {
            mcp_internal_error(format!("Bandcamp error: {error}"))
        }
        LookupWorkflowError::Serialize(error) => mcp_internal_error(error.to_string()),
    })?;

    ok_json(&result.into_output())
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
        "stage": "cache_write",
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
    cache_tx: &tokio::sync::mpsc::Sender<CacheWriteRequest<EnrichmentCacheWrite>>,
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
                    "stage": "semaphore",
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
                            "stage": "serialize",
                            "error": format!("Serialize error: {e}"),
                        })],
                    );
                }
            };
            match acknowledge_enrichment_cache_write(
                cache_tx,
                EnrichmentCacheWrite {
                    provider: match provider {
                        "beatport" => crate::types::Provider::Beatport,
                        "bandcamp" => crate::types::Provider::Bandcamp,
                        _ => unreachable!("provider_enrich_fut only supports Beatport/Bandcamp"),
                    },
                    norm_artist,
                    norm_title,
                    norm_album: None,
                    match_quality: Some(quality.to_string()),
                    response_json: Some(json_str),
                },
                provider,
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
            match acknowledge_enrichment_cache_write(
                cache_tx,
                EnrichmentCacheWrite {
                    provider: match provider {
                        "beatport" => crate::types::Provider::Beatport,
                        "bandcamp" => crate::types::Provider::Bandcamp,
                        _ => unreachable!("provider_enrich_fut only supports Beatport/Bandcamp"),
                    },
                    norm_artist,
                    norm_title,
                    norm_album: None,
                    match_quality: Some("none".to_string()),
                    response_json: None,
                },
                provider,
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
                "stage": "lookup",
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

fn enrichment_completion_flags(
    store_conn: &rusqlite::Connection,
    tracks: &[crate::types::Track],
    providers: &[crate::types::Provider],
) -> Result<Vec<bool>, rusqlite::Error> {
    let normalized: Vec<_> = tracks
        .iter()
        .map(|track| {
            (
                crate::normalize::normalize_for_matching(&track.artist),
                crate::normalize::normalize_for_matching(&track.title),
                normalize_discogs_album_for_cache(Some(&track.album)).unwrap_or_default(),
            )
        })
        .collect();
    let owned_keys: Vec<_> = normalized
        .iter()
        .flat_map(|(artist, title, album)| {
            providers.iter().map(move |provider| {
                (
                    provider.as_str().to_string(),
                    artist.clone(),
                    title.clone(),
                    if *provider == crate::types::Provider::Discogs {
                        album.clone()
                    } else {
                        String::new()
                    },
                )
            })
        })
        .collect();
    let key_refs: Vec<_> = owned_keys
        .iter()
        .map(|(provider, artist, title, album)| {
            (
                provider.as_str(),
                artist.as_str(),
                title.as_str(),
                album.as_str(),
            )
        })
        .collect();
    let cached = store::batch_get_enrichment(store_conn, &key_refs)?;

    Ok(normalized
        .iter()
        .map(|(artist, title, album)| {
            providers.iter().all(|provider| {
                let album = if *provider == crate::types::Provider::Discogs {
                    album.as_str()
                } else {
                    ""
                };
                cached.contains_key(&(
                    provider.as_str().to_string(),
                    artist.clone(),
                    title.clone(),
                    album.to_string(),
                ))
            })
        })
        .collect())
}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub(super) struct EnrichCacheWriteSummary {
    attempted: usize,
    succeeded: usize,
    failed: usize,
}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub(super) struct EnrichTracksSummary {
    tracks_total: usize,
    total: usize,
    enriched: usize,
    cached: usize,
    skipped: usize,
    failed: usize,
    concurrency: usize,
    cache_writes: EnrichCacheWriteSummary,
}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub(super) struct EnrichTracksOutput {
    summary: EnrichTracksSummary,
    failures: Vec<serde_json::Value>,
    page: BatchPage,
}

fn enrich_page_summary_counts(
    page: &BatchPage,
    provider_count: usize,
    selected_cached: usize,
) -> (usize, usize, usize) {
    let tracks_total = page.examined_tracks;
    let total = tracks_total.saturating_mul(provider_count);
    let fully_cached = page.fully_cached_skipped.saturating_mul(provider_count);
    let cached = selected_cached.saturating_add(fully_cached);
    (tracks_total, total, cached)
}

fn enrichment_join_failures(
    track_id: &str,
    artist: &str,
    title: &str,
    providers: &[crate::types::Provider],
    stage: &str,
    error: &str,
) -> Vec<serde_json::Value> {
    providers
        .iter()
        .map(|provider| {
            serde_json::json!({
                "track_id": track_id,
                "artist": artist,
                "title": title,
                "provider": provider.as_str(),
                "stage": stage,
                "error": error,
            })
        })
        .collect()
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
    cache_tx: tokio::sync::mpsc::Sender<CacheWriteRequest<EnrichmentCacheWrite>>,
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

    let stages = provider_stages(&providers);
    let want_discogs = stages.contains(
        &crate::application::enrichment::model::HydrationStage::Lookup(
            crate::types::Provider::Discogs,
        ),
    );
    let want_beatport = stages.contains(
        &crate::application::enrichment::model::HydrationStage::Lookup(
            crate::types::Provider::Beatport,
        ),
    );
    let want_bandcamp = stages.contains(
        &crate::application::enrichment::model::HydrationStage::Lookup(
            crate::types::Provider::Bandcamp,
        ),
    );

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
                        "stage": "auth",
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
                                    "stage": "serialize",
                                    "error": format!("Serialize error: {e}"),
                                })],
                                None,
                            );
                        }
                    };
                    match acknowledge_enrichment_cache_write(
                        &cache_tx,
                        EnrichmentCacheWrite {
                            provider: crate::types::Provider::Discogs,
                            norm_artist,
                            norm_title,
                            norm_album,
                            match_quality: Some(quality),
                            response_json: Some(json_str),
                        },
                        "discogs enrichment",
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
                    match acknowledge_enrichment_cache_write(
                        &cache_tx,
                        EnrichmentCacheWrite {
                            provider: crate::types::Provider::Discogs,
                            norm_artist,
                            norm_title,
                            norm_album,
                            match_quality: Some("none".to_string()),
                            response_json: None,
                        },
                        "discogs enrichment",
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
                                "stage": "auth",
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
                                "stage": "lookup",
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
    let store_path = server.cache_store_path();

    // Initialize/migrate the store without holding its MutexGuard alongside
    // the Rekordbox guard. Pending selection uses a dedicated read-only
    // connection, avoiding cross-tool lock-order deadlocks.
    {
        let _store_guard = server.cache_store_conn()?;
    }

    let selection = {
        let store_conn = if skip_cached && !force_refresh {
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
                50,
                200,
                false,
                |tracks| {
                    enrichment_completion_flags(store_conn, tracks, &providers).map_err(cache_error)
                },
            )?
        } else {
            resolve_pending_tracks(
                &conn,
                params.track_ids.as_deref(),
                params.playlist_id.as_deref(),
                params.filters,
                params.max_tracks,
                params.offset,
                50,
                200,
                false,
                |tracks| Ok(vec![false; tracks.len()]),
            )?
        }
    };
    let tracks = selection.selected;
    let page = selection.page;

    let concurrency = params.concurrency.map_or(4, |n| n.clamp(1, 8)) as usize;

    let (cache_tx, cache_rx) =
        tokio::sync::mpsc::channel::<CacheWriteRequest<EnrichmentCacheWrite>>(concurrency * 4);
    let writer_store_path = store_path.clone();
    let writer_handle = tokio::task::spawn_blocking(move || {
        run_enrichment_cache_writer(&writer_store_path, cache_rx)
    });

    let (auth_fail_tx, auth_fail_rx) = tokio::sync::watch::channel(false);
    let auth_fail_tx = std::sync::Arc::new(auth_fail_tx);
    let auth_fail_rx = std::sync::Arc::new(auth_fail_rx);

    let beatport_sem = std::sync::Arc::new(tokio::sync::Semaphore::new(2));
    let bandcamp_sem = std::sync::Arc::new(tokio::sync::Semaphore::new(2));
    let worker_report = run_bounded_workers(
        "enrich track worker task",
        tracks.clone(),
        concurrency,
        tokio_util::sync::CancellationToken::new(),
        |track| (track.id.clone(), track.artist.clone(), track.title.clone()),
        {
            let server = server.clone();
            let providers = providers.clone();
            let store_path = store_path.clone();
            let cache_tx = cache_tx.clone();
            let beatport_sem = beatport_sem.clone();
            let bandcamp_sem = bandcamp_sem.clone();
            let auth_fail_rx = auth_fail_rx.clone();
            let auth_fail_tx = auth_fail_tx.clone();
            move |track: crate::types::Track| {
                let server = server.clone();
                let providers = providers.clone();
                let store_path = store_path.clone();
                let cache_tx = cache_tx.clone();
                let beatport_sem = beatport_sem.clone();
                let bandcamp_sem = bandcamp_sem.clone();
                let auth_fail_rx = auth_fail_rx.clone();
                let auth_fail_tx = auth_fail_tx.clone();
                async move {
                    let norm_artist = crate::normalize::normalize_for_matching(&track.artist);
                    let norm_title = crate::normalize::normalize_for_matching(&track.title);
                    let norm_album = normalize_discogs_album_for_cache(Some(&track.album));
                    let result = enrich_single_track(
                        server,
                        track.id,
                        track.artist,
                        track.title,
                        track.album,
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
                    HydrationWorkerCompletion::completed(result)
                }
            }
        },
    )
    .await;

    let mut progress = BatchProgress::new();
    for (_, track_result) in worker_report.completed {
        progress.processed += track_result.processed;
        progress.cached += track_result.cached;
        progress.skipped += track_result.skipped;
        progress.failures.extend(track_result.failures);
    }
    for failure in worker_report.join_failures {
        let (track_id, artist, title) = failure.identity;
        progress.failures.extend(enrichment_join_failures(
            &track_id,
            &artist,
            &title,
            &providers,
            "task_join",
            &format!("Task panicked: {}", failure.error),
        ));
    }

    drop(cache_tx);
    let cache_write_report = match writer_handle.await {
        Ok(report) => {
            debug_assert_eq!(report.attempted, report.succeeded + report.failed);
            report
        }
        Err(e) => {
            for track in &tracks {
                progress.failures.extend(enrichment_join_failures(
                    &track.id,
                    &track.artist,
                    &track.title,
                    &providers,
                    "cache_writer_join",
                    &format!("Cache writer task failed: {e}"),
                ));
            }
            EnrichmentCacheWriterReport::default()
        }
    };

    let (tracks_total, total, cached) =
        enrich_page_summary_counts(&page, providers.len(), progress.cached);
    let result = EnrichTracksOutput {
        summary: EnrichTracksSummary {
            tracks_total,
            total,
            enriched: progress.processed,
            cached,
            skipped: progress.skipped,
            failed: progress.failures.len(),
            concurrency,
            cache_writes: EnrichCacheWriteSummary {
                attempted: cache_write_report.attempted,
                succeeded: cache_write_report.succeeded,
                failed: cache_write_report.failed,
            },
        },
        failures: progress.failures,
        page,
    };
    ok_structured_json(result)
}

#[cfg(test)]
mod pending_page_tests {
    use super::*;

    fn track(id: &str, album: &str) -> crate::types::Track {
        crate::types::Track {
            id: id.to_string(),
            title: "Shared Title".to_string(),
            artist: "Shared Artist".to_string(),
            album: album.to_string(),
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
            file_path: String::new(),
            play_count: 0,
            bit_rate: 0,
            sample_rate: 0,
            file_kind: crate::types::FileKind::Unknown(0),
            date_added: String::new(),
            position: None,
            played_at: None,
        }
    }

    fn store() -> (tempfile::TempDir, rusqlite::Connection) {
        let dir = tempfile::tempdir().expect("temporary store directory should create");
        let path = dir.path().join("store.sqlite3");
        let conn = crate::store::open(path.to_str().expect("store path should be UTF-8"))
            .expect("temporary store should open");
        (dir, conn)
    }

    fn cache(conn: &rusqlite::Connection, provider: &str, album: Option<&str>, quality: &str) {
        crate::store::set_enrichment(
            conn,
            provider,
            &crate::normalize::normalize_for_matching("Shared Artist"),
            &crate::normalize::normalize_for_matching("Shared Title"),
            album,
            Some(quality),
            None,
        )
        .expect("cache fixture should write");
    }

    #[test]
    fn enrich_tracks_pending_page_uses_exact_album_and_no_match_is_complete() {
        let (_dir, conn) = store();
        let tracks = vec![
            track("release-a", "Release A"),
            track("release-b", "Release B"),
        ];
        let album_a = crate::normalize::normalize_for_matching("Release A");
        cache(&conn, "discogs", Some(&album_a), "none");
        cache(&conn, "beatport", None, "exact");

        let complete = enrichment_completion_flags(
            &conn,
            &tracks,
            &[
                crate::types::Provider::Discogs,
                crate::types::Provider::Beatport,
            ],
        )
        .expect("completion lookup should succeed");
        assert_eq!(complete, [true, false]);

        let selection = pending_batch_page(&tracks, 0, 1, |_| Ok(complete.clone()))
            .expect("pending page should resolve");
        assert_eq!(selection.selected[0].id, "release-b");
        assert_eq!(
            selection.page,
            BatchPage {
                matched_tracks: 2,
                start_offset: 0,
                examined_tracks: 2,
                selected_tracks: 1,
                fully_cached_skipped: 1,
                next_offset: None,
                has_more: false,
            }
        );
        assert_eq!(
            enrich_page_summary_counts(&selection.page, 2, 1),
            (2, 4, 3),
            "the inspected page should count one no-match-complete track and the selected track's cached provider"
        );
    }

    #[test]
    fn enrich_tracks_pending_page_keeps_error_and_partial_provider_work_pending() {
        let (_dir, conn) = store();
        let tracks = vec![track("error", "Error Album")];
        let album = crate::normalize::normalize_for_matching("Error Album");
        cache(&conn, "discogs", Some(&album), "error");
        cache(&conn, "beatport", None, "exact");

        let complete = enrichment_completion_flags(
            &conn,
            &tracks,
            &[
                crate::types::Provider::Discogs,
                crate::types::Provider::Beatport,
                crate::types::Provider::Bandcamp,
            ],
        )
        .expect("completion lookup should succeed");
        assert_eq!(complete, [false]);

        let selection = pending_batch_page(&tracks, 0, 1, |_| Ok(complete.clone()))
            .expect("partial-provider page should resolve");
        assert_eq!(
            enrich_page_summary_counts(&selection.page, 3, 1),
            (1, 3, 1),
            "error entries stay pending while successful provider cache hits remain counted"
        );
    }

    #[test]
    fn enrich_tracks_pending_page_force_refresh_marks_every_candidate_pending() {
        let tracks = vec![track("first", "A"), track("second", "B")];
        let selection = pending_batch_page(&tracks, 0, 1, |candidates| {
            Ok(vec![false; candidates.len()])
        })
        .expect("refresh page should resolve");
        assert_eq!(selection.selected[0].id, "first");
        assert_eq!(selection.page.next_offset, Some(1));
        assert!(selection.page.has_more);
    }

    #[tokio::test]
    async fn enrich_tracks_pending_page_writer_failure_retains_retry_identity() {
        let (cache_tx, cache_rx) = tokio::sync::mpsc::channel(1);
        drop(cache_rx);
        let (_, _, failures) = provider_enrich_fut(
            "beatport",
            true,
            "retry-track",
            "Retry Artist",
            "Retry Title",
            "retry artist".to_string(),
            "retry title".to_string(),
            std::sync::Arc::new(tokio::sync::Semaphore::new(1)),
            &cache_tx,
            async {
                Ok::<Option<serde_json::Value>, String>(Some(serde_json::json!({"ok": true})))
            },
            |_| "exact",
        )
        .await;

        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0]["track_id"], "retry-track");
        assert_eq!(failures[0]["provider"], "beatport");
        assert_eq!(failures[0]["stage"], "cache_write");
    }

    #[test]
    fn enrich_tracks_pending_page_provider_or_cache_policy_change_requires_restart() {
        let (_dir, conn) = store();
        let tracks = vec![track("first", "A"), track("second", "B")];
        for album in ["A", "B"] {
            let normalized = crate::normalize::normalize_for_matching(album);
            cache(&conn, "discogs", Some(&normalized), "none");
        }
        cache(&conn, "bandcamp", None, "none");

        let discogs_only =
            enrichment_completion_flags(&conn, &tracks, &[crate::types::Provider::Discogs])
                .expect("single-provider completion should resolve");
        assert_eq!(discogs_only, [true, true]);

        let expanded = enrichment_completion_flags(
            &conn,
            &tracks,
            &[
                crate::types::Provider::Discogs,
                crate::types::Provider::Bandcamp,
            ],
        )
        .expect("expanded-provider completion should resolve");
        assert_eq!(expanded, [true, true]);

        // Bandcamp's artist/title key is shared in this fixture. Delete it to
        // make both earlier candidates pending under the expanded policy.
        conn.execute(
            "DELETE FROM enrichment_cache WHERE provider = 'bandcamp'",
            [],
        )
        .expect("Bandcamp fixture should clear");
        let expanded = enrichment_completion_flags(
            &conn,
            &tracks,
            &[
                crate::types::Provider::Discogs,
                crate::types::Provider::Bandcamp,
            ],
        )
        .expect("expanded-provider completion should resolve");
        assert_eq!(expanded, [false, false]);

        let completion = |candidates: &[crate::types::Track]| {
            Ok(candidates
                .iter()
                .map(|track| expanded[usize::from(track.id == "second")])
                .collect())
        };
        let stale_offset = pending_batch_page(&tracks, 1, 1, completion)
            .expect("changed-policy offset page should resolve");
        assert_eq!(stale_offset.selected[0].id, "second");
        let restarted = pending_batch_page(&tracks, 0, 1, completion)
            .expect("changed-policy restart should resolve");
        assert_eq!(restarted.selected[0].id, "first");

        let forced = pending_batch_page(&tracks, 0, 1, |candidates| {
            Ok(vec![false; candidates.len()])
        })
        .expect("force-refresh restart should resolve");
        assert_eq!(forced.selected[0].id, "first");
    }

    #[test]
    fn enrich_tracks_pending_page_join_failure_enumerates_provider_identities() {
        let failures = enrichment_join_failures(
            "retry-track",
            "Retry Artist",
            "Retry Title",
            &[
                crate::types::Provider::Discogs,
                crate::types::Provider::Beatport,
                crate::types::Provider::Bandcamp,
            ],
            "cache_writer_join",
            "sentinel join failure",
        );
        assert_eq!(failures.len(), 3);
        assert_eq!(failures[0]["track_id"], "retry-track");
        assert_eq!(failures[0]["provider"], "discogs");
        assert_eq!(failures[1]["provider"], "beatport");
        assert_eq!(failures[2]["provider"], "bandcamp");
        assert!(
            failures
                .iter()
                .all(|failure| failure["stage"] == "cache_writer_join")
        );
    }
}

#[cfg(test)]
mod cache_write_tests {
    use super::*;
    use std::future::Future;
    use std::time::Duration;

    type EnrichCacheWrite = EnrichmentCacheWrite;
    type EnrichCacheWriteMsg = CacheWriteRequest<EnrichmentCacheWrite>;
    type EnrichCacheWriterReport = EnrichmentCacheWriterReport;

    const OUTER_TIMEOUT: Duration = Duration::from_secs(5);
    const STEP_TIMEOUT: Duration = Duration::from_secs(2);

    fn test_write(title: &str) -> EnrichCacheWrite {
        EnrichCacheWrite {
            provider: crate::types::Provider::Beatport,
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

    async fn write_enrichment_cache(
        sender: &tokio::sync::mpsc::Sender<EnrichCacheWriteMsg>,
        write: EnrichCacheWrite,
    ) -> Result<(), String> {
        acknowledge_enrichment_cache_write(sender, write, "enrichment").await
    }

    fn run_enrich_cache_writer(
        store_path: &str,
        receiver: tokio::sync::mpsc::Receiver<EnrichCacheWriteMsg>,
    ) -> EnrichCacheWriterReport {
        run_enrichment_cache_writer(store_path, receiver)
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
        let (sender, mut receiver) = tokio::sync::mpsc::channel::<EnrichCacheWriteMsg>(1);
        let writer = tokio::spawn(async move {
            let message = bounded("ack channel receive", receiver.recv())
                .await?
                .ok_or_else(|| "ack channel closed before a message arrived".to_string())?;
            if let Some(result) = acknowledgement {
                message
                    .acknowledgement
                    .send(result)
                    .map_err(|_| "ack receiver dropped unexpectedly".to_string())?;
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
        assert!(
            result
                .expect_err("closed queue should fail")
                .starts_with("enrichment cache queue send failed:")
        );
    }

    #[tokio::test]
    async fn enrich_cache_ack_canceled_sender() {
        let result = tokio::time::timeout(OUTER_TIMEOUT, exercise_ack(None))
            .await
            .expect("ack cancellation scenario should finish within five seconds")
            .expect("ack cancellation harness should finish cleanly");
        assert_eq!(
            result,
            Err("enrichment cache acknowledgement canceled".to_string())
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
                self.sender().send(CacheWriteRequest {
                    payload: write,
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
