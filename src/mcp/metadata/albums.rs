use rmcp::ErrorData as McpError;
use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::adapters::rekordbox as db;
use crate::application::metadata::backfill::{scan_albums, stage_suggestions};
use crate::application::metadata::enrichment::{
    MetadataAutoEnrichmentReport, MetadataEnrichmentProvider, MetadataEnrichmentRequest,
    MetadataEnrichmentWriterSession, run_metadata_provider,
};
use crate::mcp::{ReklawdboxServer, db_error, lookup_bandcamp_remote, mcp_internal_error, ok_json};

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

    let mut auto_enrichment = MetadataAutoEnrichmentReport::default();

    if auto_enrich && !scan.uncached_bandcamp.is_empty() {
        let requests: Vec<_> = std::mem::take(&mut scan.uncached_bandcamp)
            .into_iter()
            .map(MetadataEnrichmentRequest::from)
            .collect();
        tracing::info!(
            count = requests.len(),
            "auto_enrich: fetching Bandcamp for uncached album tracks"
        );

        use crate::mcp::enrichment::HasScore;

        // Ensure the DB exists and is migrated before spawning the writer.
        {
            let _conn = server.cache_store_conn()?;
        }
        let writer = MetadataEnrichmentWriterSession::start(
            server.cache_store_path(),
            MetadataEnrichmentProvider::Bandcamp,
            requests
                .first()
                .expect("album auto-enrichment requests should be non-empty"),
        );
        let lookup_server = server.clone();
        let provider_report = run_metadata_provider(
            MetadataEnrichmentProvider::Bandcamp,
            requests,
            writer.sender(),
            move |request| {
                let server = lookup_server.clone();
                async move {
                    lookup_bandcamp_remote(&server, &request.raw_artist, &request.raw_title).await
                }
            },
            |result| {
                if result.score() >= 90 {
                    "exact"
                } else {
                    "fuzzy"
                }
            },
        )
        .await;
        auto_enrichment = writer
            .finish(provider_report)
            .await
            .map_err(mcp_internal_error)?;

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
            serde_json::json!(auto_enrichment.matched),
        );
        result.as_object_mut().unwrap().insert(
            "auto_enrichment".to_string(),
            serde_json::json!(auto_enrichment),
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
