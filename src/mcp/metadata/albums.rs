use rmcp::ErrorData as McpError;
use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;

#[allow(unused_imports)]
use crate::application::metadata::backfill::{scan_albums, stage_suggestions};
use crate::db;
use crate::mcp::{ReklawdboxServer, db_error, mcp_internal_error, ok_json};
use crate::store;

#[derive(Debug, Deserialize, JsonSchema)]
pub(in crate::mcp) struct BackfillAlbumsParams {
    #[schemars(description = "Preview changes without staging (default false)")]
    pub dry_run: Option<bool>,
    #[schemars(
        description = "Automatically enrich uncached tracks via Bandcamp before backfilling (default false)."
    )]
    pub auto_enrich: Option<bool>,
}

pub(in crate::mcp) async fn handle_backfill_albums(
    server: &ReklawdboxServer,
    params: BackfillAlbumsParams,
) -> Result<CallToolResult, McpError> {
    let dry_run = params.dry_run.unwrap_or(false);
    let auto_enrich = params.auto_enrich.unwrap_or(false);

    let (tracks, mut scan) = {
        let rb_conn = server.rekordbox_conn()?;
        let search_params = db::SearchParams {
            exclude_samples: true,
            ..Default::default()
        };
        let tracks = db::search_tracks_unbounded(&rb_conn, &search_params).map_err(db_error)?;
        drop(rb_conn);

        let store_conn = server.cache_store_conn()?;
        let scan = scan_albums(&store_conn, &tracks);
        drop(store_conn);
        (tracks, scan)
    };

    let mut auto_enriched = 0usize;

    if auto_enrich && !scan.uncached_bandcamp.is_empty() {
        let to_enrich: Vec<_> = std::mem::take(&mut scan.uncached_bandcamp);
        let total = to_enrich.len();
        tracing::info!(
            count = total,
            "auto_enrich: fetching Bandcamp for uncached album tracks"
        );

        use crate::mcp::enrichment::HasScore;

        let store_path = server.cache_store_path();

        // Ensure the DB exists and is migrated before spawning the writer.
        {
            let _conn = server.cache_store_conn()?;
        }

        let (cache_tx, mut cache_rx) =
            tokio::sync::mpsc::channel::<(String, String, Option<String>, Option<String>)>(16);
        let writer_store_path = store_path.clone();
        let writer_handle = tokio::task::spawn_blocking(move || {
            let conn = match store::open(&writer_store_path) {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!("album auto-enrich writer: failed to open store: {e}");
                    return;
                }
            };
            while let Some((norm_artist, norm_title, quality, json)) = cache_rx.blocking_recv() {
                if let Err(e) = store::set_enrichment(
                    &conn,
                    "bandcamp",
                    &norm_artist,
                    &norm_title,
                    None,
                    quality.as_deref(),
                    json.as_deref(),
                ) {
                    tracing::error!(
                        "album auto-enrich writer: failed to write bandcamp for {norm_artist}/{norm_title}: {e}"
                    );
                }
            }
        });

        let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(4));
        let mut handles = Vec::with_capacity(total);

        for (norm_artist, norm_title, raw_artist, raw_title) in to_enrich {
            let permit = sem
                .clone()
                .acquire_owned()
                .await
                .map_err(|e| mcp_internal_error(format!("Semaphore error: {e}")))?;

            let server = server.clone();
            let cache_tx = cache_tx.clone();

            handles.push(tokio::spawn(async move {
                let _permit = permit;
                let enriched: usize;
                match crate::bandcamp::lookup(
                    &server.context.enrichment.http,
                    &raw_artist,
                    &raw_title,
                )
                .await
                {
                    Ok(result) => {
                        let (quality, json) = match &result {
                            Some(r) => {
                                let q = if r.score() >= 90 { "exact" } else { "fuzzy" };
                                let j = serde_json::to_string(r).ok();
                                (Some(q.to_string()), j)
                            }
                            None => (Some("none".to_string()), None),
                        };
                        enriched = if result.is_some() { 1 } else { 0 };
                        let _ = cache_tx
                            .send((norm_artist, norm_title, quality, json))
                            .await;
                    }
                    Err(e) => {
                        tracing::warn!(
                            artist = raw_artist.as_str(),
                            "Bandcamp auto-enrich failed: {e}"
                        );
                        enriched = 0;
                    }
                }
                enriched
            }));
        }

        drop(cache_tx);

        for handle in handles {
            match handle.await {
                Ok(n) => auto_enriched += n,
                Err(e) => tracing::warn!("album auto-enrich task panicked: {e}"),
            }
        }

        if let Err(e) = writer_handle.await {
            tracing::warn!("album backfill cache writer task failed: {e}");
        }

        let store_conn = server.cache_store_conn()?;
        scan = scan_albums(&store_conn, &tracks);
        drop(store_conn);
    }

    let (staged_count, pending) =
        stage_suggestions(&server.context.mutation.changes, scan.to_stage, dry_run);

    let mut result = serde_json::json!({
        "summary": {
            "total_scanned": tracks.len(),
            "filled": scan.filled,
            "already_set": scan.already_set,
            "no_source": scan.no_source,
            "skipped_noise": scan.skipped_noise,
            "filled_by_source": {
                "file_tags": scan.filled_by_source.file_tags,
                "folder_path": scan.filled_by_source.folder_path,
                "bandcamp": scan.filled_by_source.bandcamp,
                "discogs": scan.filled_by_source.discogs,
            },
        },
        "staged": staged_count,
        "total_pending": pending,
        "dry_run": dry_run,
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
    use crate::application::metadata::backfill::{album_from_folder_path, is_noise_album};

    #[test]
    fn album_from_folder_basic() {
        assert_eq!(
            album_from_folder_path("/music/Artist/Album Name (2019)/01 Track.wav"),
            Some("Album Name".to_string())
        );
    }

    #[test]
    fn album_from_folder_strips_edition() {
        assert_eq!(
            album_from_folder_path("/music/The XX/Coexist (Japanese Edition) (2012)/track.flac"),
            Some("Coexist".to_string())
        );
    }

    #[test]
    fn album_from_folder_strips_soundtrack() {
        assert_eq!(
            album_from_folder_path(
                "/music/VA/Hell Or High Water (Original Motion Picture Soundtrack) (2016)/t.wav"
            ),
            Some("Hell Or High Water".to_string())
        );
    }

    #[test]
    fn album_from_folder_strips_deluxe() {
        assert_eq!(
            album_from_folder_path("/music/XX/I See You (Deluxe Edition) (2017)/track.flac"),
            Some("I See You".to_string())
        );
    }

    #[test]
    fn album_from_folder_no_year() {
        assert_eq!(album_from_folder_path("/music/play/play1/track.wav"), None,);
    }

    #[test]
    fn album_from_folder_preserves_non_qualifier_parens() {
        assert_eq!(
            album_from_folder_path("/music/Artist/Music (Is My Life) (2020)/track.flac"),
            Some("Music (Is My Life)".to_string())
        );
    }

    #[test]
    fn noise_filter() {
        assert!(is_noise_album("Some Track", "Some Track", "Artist"));
        assert!(is_noise_album("Artist Name", "Track", "Artist Name"));
        assert!(!is_noise_album("Album Name", "Track", "Artist"));
    }
}
