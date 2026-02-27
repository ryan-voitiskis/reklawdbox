use rmcp::model::{CallToolResult, Content};
use rmcp::ErrorData as McpError;

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
            let result = lookup_output_with_cache_metadata(
                result,
                true,
                Some(cached.created_at.as_str()),
            );
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
            let json =
                serde_json::to_string(r).map_err(|e| mcp_internal_error(format!("{e}")))?;
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
            let result = lookup_output_with_cache_metadata(
                result,
                true,
                Some(cached.created_at.as_str()),
            );
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
            let json =
                serde_json::to_string(r).map_err(|e| mcp_internal_error(format!("{e}")))?;
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
            &ResolveTracksOpts {
                default_max_tracks: Some(50),
                max_tracks_cap: Some(200),
                exclude_samplers: false,
            },
        )?
    };

    let total_tracks = tracks.len();
    let total = total_tracks.saturating_mul(providers.len());

    let mut progress = BatchProgress::new();
    let mut discogs_auth_error: Option<String> = None;

    for track in &tracks {
        let norm_artist = crate::normalize::normalize_for_matching(&track.artist);
        let norm_title = crate::normalize::normalize_for_matching(&track.title);

        for provider in &providers {
            if skip_cached && !force_refresh {
                let store_conn = server.cache_store_conn()?;
                if store::get_enrichment(
                    &store_conn,
                    provider.as_str(),
                    &norm_artist,
                    &norm_title,
                )
                .map_err(|e| mcp_internal_error(format!("Cache read error: {e}")))?
                .is_some()
                {
                    progress.cached += 1;
                    continue;
                }
            }

            match provider {
                crate::types::Provider::Discogs => {
                    if let Some(auth_err) = discogs_auth_error.clone() {
                        progress.failures.push(serde_json::json!({
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
                    match lookup_discogs_remote(server, &track.artist, &track.title, album).await {
                        Ok(Some(r)) => {
                            let json_str = serde_json::to_string(&r)
                                .map_err(|e| mcp_internal_error(format!("{e}")))?;
                            let quality = if r.fuzzy_match { "fuzzy" } else { "exact" };
                            let store_conn = server.cache_store_conn()?;
                            store::set_enrichment(
                                &store_conn,
                                provider.as_str(),
                                &norm_artist,
                                &norm_title,
                                Some(quality),
                                Some(&json_str),
                            )
                            .map_err(|e| {
                                mcp_internal_error(format!("Cache write error: {e}"))
                            })?;
                            progress.processed += 1;
                        }
                        Ok(None) => {
                            let store_conn = server.cache_store_conn()?;
                            store::set_enrichment(
                                &store_conn,
                                provider.as_str(),
                                &norm_artist,
                                &norm_title,
                                Some("none"),
                                None,
                            )
                            .map_err(|e| {
                                mcp_internal_error(format!("Cache write error: {e}"))
                            })?;
                            progress.skipped += 1;
                        }
                        Err(e) => {
                            let error_message =
                                if let Some(remediation) = e.auth_remediation() {
                                    let msg = auth_remediation_message(remediation);
                                    discogs_auth_error = Some(msg.clone());
                                    msg
                                } else {
                                    e.to_string()
                                };
                            progress.failures.push(serde_json::json!({
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
                    match beatport::lookup(&server.state.http, &track.artist, &track.title).await {
                        Ok(Some(r)) => {
                            let json_str = serde_json::to_string(&r)
                                .map_err(|e| mcp_internal_error(format!("{e}")))?;
                            let store_conn = server.cache_store_conn()?;
                            store::set_enrichment(
                                &store_conn,
                                provider.as_str(),
                                &norm_artist,
                                &norm_title,
                                Some("exact"),
                                Some(&json_str),
                            )
                            .map_err(|e| {
                                mcp_internal_error(format!("Cache write error: {e}"))
                            })?;
                            progress.processed += 1;
                        }
                        Ok(None) => {
                            let store_conn = server.cache_store_conn()?;
                            store::set_enrichment(
                                &store_conn,
                                provider.as_str(),
                                &norm_artist,
                                &norm_title,
                                Some("none"),
                                None,
                            )
                            .map_err(|e| {
                                mcp_internal_error(format!("Cache write error: {e}"))
                            })?;
                            progress.skipped += 1;
                        }
                        Err(e) => {
                            progress.failures.push(serde_json::json!({
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
            "enriched": progress.processed,
            "cached": progress.cached,
            "skipped": progress.skipped,
            "failed": progress.failures.len(),
        },
        "failures": progress.failures,
    });
    let json =
        serde_json::to_string_pretty(&result).map_err(|e| mcp_internal_error(format!("{e}")))?;
    Ok(CallToolResult::success(vec![Content::text(json)]))
}
