use rmcp::ErrorData as McpError;
use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::adapters::rekordbox as db;
use crate::application::metadata::backfill::{scan_years, stage_suggestions};
use crate::application::metadata::enrichment::{
    MetadataAutoEnrichmentReport, MetadataEnrichmentProvider, MetadataEnrichmentRequest,
    MetadataEnrichmentWriterSession, run_metadata_provider,
};
use crate::mcp::{
    ReklawdboxServer, db_error, lookup_bandcamp_remote, lookup_musicbrainz_remote,
    mcp_internal_error, ok_json,
};

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

    let mut auto_enrichment = MetadataAutoEnrichmentReport::default();

    if auto_enrich && (!scan.uncached_bandcamp.is_empty() || !scan.uncached_musicbrainz.is_empty())
    {
        use crate::mcp::enrichment::HasScore;

        let bandcamp_requests: Vec<_> = std::mem::take(&mut scan.uncached_bandcamp)
            .into_iter()
            .map(MetadataEnrichmentRequest::from)
            .collect();
        let musicbrainz_requests: Vec<_> = std::mem::take(&mut scan.uncached_musicbrainz)
            .into_iter()
            .map(MetadataEnrichmentRequest::from)
            .collect();
        let total = bandcamp_requests.len() + musicbrainz_requests.len();
        tracing::info!(
            bandcamp = bandcamp_requests.len(),
            musicbrainz = musicbrainz_requests.len(),
            "auto_enrich: fetching for {total} uncached year-zero tracks"
        );

        let (writer_provider, writer_request) = bandcamp_requests
            .first()
            .map(|request| (MetadataEnrichmentProvider::Bandcamp, request))
            .or_else(|| {
                musicbrainz_requests
                    .first()
                    .map(|request| (MetadataEnrichmentProvider::MusicBrainz, request))
            })
            .expect("year auto-enrichment requests should be non-empty");
        let writer = MetadataEnrichmentWriterSession::start(
            server.cache_store_path(),
            writer_provider,
            writer_request,
        );
        let bandcamp_server = server.clone();
        let bandcamp = run_metadata_provider(
            MetadataEnrichmentProvider::Bandcamp,
            bandcamp_requests,
            writer.sender(),
            move |request| {
                let server = bandcamp_server.clone();
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
        );
        let musicbrainz_server = server.clone();
        let musicbrainz = run_metadata_provider(
            MetadataEnrichmentProvider::MusicBrainz,
            musicbrainz_requests,
            writer.sender(),
            move |request| {
                let server = musicbrainz_server.clone();
                async move {
                    lookup_musicbrainz_remote(&server, &request.raw_artist, &request.raw_title)
                        .await
                }
            },
            |result| {
                if result.score() >= 90 {
                    "exact"
                } else {
                    "fuzzy"
                }
            },
        );
        let (bandcamp, musicbrainz) = tokio::join!(bandcamp, musicbrainz);
        let mut provider_report = bandcamp;
        provider_report.absorb(musicbrainz);
        auto_enrichment = writer
            .finish(provider_report)
            .await
            .map_err(mcp_internal_error)?;

        // Re-scan only after the acknowledged writer has terminated.
        let store_conn = server.cache_store_conn()?;
        scan = scan_years(&store_conn, &tracks);
        drop(store_conn);
    }

    let filled = scan.filled_file_tags
        + scan.filled_folder_path
        + scan.filled_discogs
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
                "musicbrainz": scan.filled_musicbrainz,
                "bandcamp": scan.filled_bandcamp,
            },
            "already_set": scan.already_set,
            "conflicts": scan.conflicts.len(),
            "remaining_year_zero": scan.remaining_year_zero.len(),
            "remaining_uncached_providers": {
                "no_discogs": scan.remaining_no_discogs,
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
