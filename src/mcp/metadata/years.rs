use rmcp::ErrorData as McpError;
use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::mcp::{
    ReklawdboxServer, db_error, lookup_bandcamp_remote, lookup_musicbrainz_remote,
    mcp_internal_error, ok_json,
};
// Temporary compatibility consumer for the legacy classify-handler re-export; retire in Plan 046.
#[allow(unused_imports)]
use crate::application::metadata::backfill::{scan_years, stage_suggestions};
use crate::db;
#[allow(unused_imports)]
use crate::mcp::classification::parse_response_json as _;
use crate::store;

#[derive(Debug, Deserialize, JsonSchema)]
pub(in crate::mcp) struct BackfillYearsParams {
    #[schemars(description = "Preview changes without staging (default false)")]
    pub dry_run: Option<bool>,
    #[schemars(
        description = "Automatically enrich remaining year-zero tracks via Bandcamp and MusicBrainz before re-scanning (default false). Fetches data for tracks missing from enrichment caches, then re-runs the year cascade."
    )]
    pub auto_enrich: Option<bool>,
}

pub(in crate::mcp) async fn handle_backfill_years(
    server: &ReklawdboxServer,
    params: BackfillYearsParams,
) -> Result<CallToolResult, McpError> {
    let dry_run = params.dry_run.unwrap_or(false);
    let auto_enrich = params.auto_enrich.unwrap_or(false);

    // Synchronous DB + cache work in a block to drop MutexGuard before any .await.
    let (tracks, mut scan) = {
        let rb_conn = server.rekordbox_conn()?;
        let search_params = db::SearchParams {
            exclude_samples: true,
            ..Default::default()
        };
        let tracks = db::search_tracks_unbounded(&rb_conn, &search_params).map_err(db_error)?;
        drop(rb_conn);

        let store_conn = server.cache_store_conn()?;
        let scan = scan_years(&store_conn, &tracks);
        drop(store_conn);
        (tracks, scan)
    };

    let mut auto_enriched = 0usize;

    if auto_enrich && (!scan.uncached_bandcamp.is_empty() || !scan.uncached_musicbrainz.is_empty())
    {
        use crate::mcp::enrichment::HasScore;

        let bc_tracks: Vec<_> = std::mem::take(&mut scan.uncached_bandcamp);
        let mb_tracks: Vec<_> = std::mem::take(&mut scan.uncached_musicbrainz);
        let total = bc_tracks.len() + mb_tracks.len();
        tracing::info!(
            bandcamp = bc_tracks.len(),
            musicbrainz = mb_tracks.len(),
            "auto_enrich: fetching for {total} uncached year-zero tracks"
        );

        // Shared channel + spawn_blocking writer for cache writes.
        let (cache_tx, mut cache_rx) =
            tokio::sync::mpsc::channel::<(String, String, String, Option<String>, Option<String>)>(
                32,
            );

        let writer_store_path = server.cache_store_path();
        let writer_handle = tokio::task::spawn_blocking(move || {
            let conn = match store::open(&writer_store_path) {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!("Year backfill cache writer: failed to open store: {e}");
                    return;
                }
            };
            while let Some((provider, norm_artist, norm_title, match_quality, response_json)) =
                cache_rx.blocking_recv()
            {
                if let Err(e) = store::set_enrichment(
                    &conn,
                    &provider,
                    &norm_artist,
                    &norm_title,
                    None,
                    match_quality.as_deref(),
                    response_json.as_deref(),
                ) {
                    tracing::error!(
                        "Year backfill cache writer: failed to write {provider} for {norm_artist}/{norm_title}: {e}"
                    );
                }
            }
        });

        // Two concurrent dispatch futures — one per provider.
        let bc_future = {
            let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(4));
            let mut handles = Vec::with_capacity(bc_tracks.len());

            for (norm_artist, norm_title, raw_artist, raw_title) in bc_tracks {
                let permit = sem
                    .clone()
                    .acquire_owned()
                    .await
                    .map_err(|e| mcp_internal_error(format!("Semaphore error: {e}")))?;
                let server = server.clone();
                let cache_tx = cache_tx.clone();

                handles.push(tokio::spawn(async move {
                    let enriched =
                        match lookup_bandcamp_remote(&server, &raw_artist, &raw_title).await {
                            Ok(Some(r)) => {
                                let json_str = match serde_json::to_string(&r) {
                                    Ok(j) => j,
                                    Err(e) => {
                                        tracing::warn!(
                                            provider = "bandcamp",
                                            artist = norm_artist.as_str(),
                                            "serialization failed: {e}"
                                        );
                                        drop(permit);
                                        return 1usize;
                                    }
                                };
                                let quality = if r.score() >= 90 {
                                    "exact".to_string()
                                } else {
                                    "fuzzy".to_string()
                                };
                                let _ = cache_tx
                                    .send((
                                        "bandcamp".to_string(),
                                        norm_artist,
                                        norm_title,
                                        Some(quality),
                                        Some(json_str),
                                    ))
                                    .await;
                                1usize
                            }
                            Ok(None) => {
                                let _ = cache_tx
                                    .send((
                                        "bandcamp".to_string(),
                                        norm_artist,
                                        norm_title,
                                        Some("none".to_string()),
                                        None,
                                    ))
                                    .await;
                                0usize
                            }
                            Err(e) => {
                                tracing::warn!(
                                    artist = raw_artist.as_str(),
                                    "Bandcamp auto-enrich failed: {e}"
                                );
                                0usize
                            }
                        };
                    drop(permit);
                    enriched
                }));
            }

            async move {
                let mut sum = 0usize;
                for handle in handles {
                    match handle.await {
                        Ok(n) => sum += n,
                        Err(e) => tracing::warn!("Bandcamp task panicked: {e}"),
                    }
                }
                sum
            }
        };

        let mb_future = {
            let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(4));
            let mut handles = Vec::with_capacity(mb_tracks.len());

            for (norm_artist, norm_title, raw_artist, raw_title) in mb_tracks {
                let permit = sem
                    .clone()
                    .acquire_owned()
                    .await
                    .map_err(|e| mcp_internal_error(format!("Semaphore error: {e}")))?;
                let server = server.clone();
                let cache_tx = cache_tx.clone();

                handles.push(tokio::spawn(async move {
                    let enriched =
                        match lookup_musicbrainz_remote(&server, &raw_artist, &raw_title).await {
                            Ok(Some(r)) => {
                                let json_str = match serde_json::to_string(&r) {
                                    Ok(j) => j,
                                    Err(e) => {
                                        tracing::warn!(
                                            provider = "musicbrainz",
                                            artist = norm_artist.as_str(),
                                            "serialization failed: {e}"
                                        );
                                        drop(permit);
                                        return 1usize;
                                    }
                                };
                                let quality = if r.score() >= 90 {
                                    "exact".to_string()
                                } else {
                                    "fuzzy".to_string()
                                };
                                let _ = cache_tx
                                    .send((
                                        "musicbrainz".to_string(),
                                        norm_artist,
                                        norm_title,
                                        Some(quality),
                                        Some(json_str),
                                    ))
                                    .await;
                                1usize
                            }
                            Ok(None) => {
                                let _ = cache_tx
                                    .send((
                                        "musicbrainz".to_string(),
                                        norm_artist,
                                        norm_title,
                                        Some("none".to_string()),
                                        None,
                                    ))
                                    .await;
                                0usize
                            }
                            Err(e) => {
                                tracing::warn!(
                                    artist = raw_artist.as_str(),
                                    "MusicBrainz auto-enrich failed: {e}"
                                );
                                0usize
                            }
                        };
                    drop(permit);
                    enriched
                }));
            }

            async move {
                let mut sum = 0usize;
                for handle in handles {
                    match handle.await {
                        Ok(n) => sum += n,
                        Err(e) => tracing::warn!("MusicBrainz task panicked: {e}"),
                    }
                }
                sum
            }
        };

        // Run both provider dispatches concurrently.
        let (bc_enriched, mb_enriched) = tokio::join!(bc_future, mb_future);
        auto_enriched = bc_enriched + mb_enriched;

        // Close channel so writer drains and exits.
        drop(cache_tx);
        if let Err(e) = writer_handle.await {
            tracing::error!("Year backfill cache writer task failed: {e}");
        }

        // Re-scan with updated cache
        let store_conn = server.cache_store_conn()?;
        scan = scan_years(&store_conn, &tracks);
        drop(store_conn);
    }

    let filled = scan.filled_file_tags
        + scan.filled_folder_path
        + scan.filled_discogs
        + scan.filled_beatport
        + scan.filled_musicbrainz
        + scan.filled_bandcamp;

    let (staged_count, pending) =
        stage_suggestions(&server.context.mutation.changes, scan.to_stage, dry_run);

    let mut result = serde_json::json!({
        "summary": {
            "total_scanned": tracks.len(),
            "filled": filled,
            "filled_by_source": {
                "file_tags": scan.filled_file_tags,
                "folder_path": scan.filled_folder_path,
                "discogs": scan.filled_discogs,
                "beatport": scan.filled_beatport,
                "musicbrainz": scan.filled_musicbrainz,
                "bandcamp": scan.filled_bandcamp,
            },
            "already_set": scan.already_set,
            "conflicts": scan.conflicts.len(),
            "remaining_year_zero": scan.remaining_year_zero.len(),
            "remaining_uncached_providers": {
                "no_discogs": scan.remaining_no_discogs,
                "no_beatport": scan.remaining_no_beatport,
                "no_musicbrainz": scan.remaining_no_musicbrainz,
                "no_bandcamp": scan.remaining_no_bandcamp,
            },
        },
        "staged": staged_count,
        "total_pending": pending,
        "dry_run": dry_run,
        "conflicts": scan.conflicts,
        "remaining_year_zero": scan.remaining_year_zero,
    });

    if auto_enrich {
        result.as_object_mut().unwrap().insert(
            "auto_enriched".to_string(),
            serde_json::json!(auto_enriched),
        );
    }

    ok_json(&result)
}

#[cfg(test)]
mod tests {
    use crate::application::metadata::backfill::{parse_year_str, year_from_folder_path};

    #[test]
    fn parse_year_str_plain() {
        assert_eq!(parse_year_str("2019"), Some(2019));
        assert_eq!(parse_year_str(" 1992 "), Some(1992));
    }

    #[test]
    fn parse_year_str_iso_date() {
        assert_eq!(parse_year_str("2019-01-15"), Some(2019));
        assert_eq!(parse_year_str("2024-12"), Some(2024));
    }

    #[test]
    fn parse_year_str_beatport_datetime() {
        assert_eq!(parse_year_str("2010-12-06T00:00:00"), Some(2010));
        assert_eq!(parse_year_str("2023-03-17T00:00:00"), Some(2023));
    }

    #[test]
    fn parse_year_str_rejects_invalid() {
        assert_eq!(parse_year_str(""), None);
        assert_eq!(parse_year_str("abc"), None);
        assert_eq!(parse_year_str("1899"), None);
        assert_eq!(parse_year_str("2100"), None);
    }

    #[test]
    fn folder_path_extracts_year_suffix() {
        assert_eq!(
            year_from_folder_path("/music/Artist/Album (2019)/01 Track.wav"),
            Some(2019)
        );
        assert_eq!(
            year_from_folder_path("/music/Artist/Album (1993)/track.flac"),
            Some(1993)
        );
    }

    #[test]
    fn folder_path_no_year() {
        assert_eq!(year_from_folder_path("/music/play/play25/Track.wav"), None);
        assert_eq!(year_from_folder_path("/music/Artist/Album/track.wav"), None);
    }

    #[test]
    fn folder_path_rejects_invalid_year() {
        assert_eq!(
            year_from_folder_path("/music/Artist/Album (1899)/track.wav"),
            None
        );
        assert_eq!(
            year_from_folder_path("/music/Artist/Album (2100)/track.wav"),
            None
        );
    }

    #[test]
    fn folder_path_ignores_non_suffix_parens() {
        // Year must be at the end in parentheses
        assert_eq!(
            year_from_folder_path("/music/Artist/(2019) Album/track.wav"),
            None
        );
    }
}
