use rmcp::ErrorData as McpError;
use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::adapters::rekordbox as db;
use crate::application::metadata::backfill::{scan_labels, stage_suggestions};
use crate::application::metadata::enrichment::{
    MetadataAutoEnrichmentReport, MetadataEnrichmentProvider, MetadataEnrichmentRequest,
    MetadataEnrichmentWriterSession, run_metadata_provider,
};
use crate::domain::metadata as normalize;
use crate::mcp::{
    OffsetPage, ReklawdboxServer, db_error, lookup_bandcamp_remote, lookup_musicbrainz_remote,
    mcp_internal_error, offset_page_bounds, ok_structured_json,
};

#[derive(Debug, Deserialize, JsonSchema)]
pub(in crate::mcp) struct BackfillLabelsParams {
    #[schemars(description = "Preview changes without staging (default false)")]
    pub dry_run: Option<bool>,
    #[schemars(
        description = "Automatically enrich uncached label keys via MusicBrainz and Bandcamp before backfilling (default false), then re-scan with Discogs > MusicBrainz > Bandcamp precedence."
    )]
    pub auto_enrich: Option<bool>,
    #[schemars(
        description = "Max conflict entries to include in the response (default 50). Use conflict_offset and conflict_page.next_offset to continue."
    )]
    pub max_conflicts: Option<usize>,
    #[schemars(description = "Offset into the stable ordered label-conflict list (default 0)")]
    pub conflict_offset: Option<usize>,
}

/// Filters out Discogs "Not On Label" entries (self-released, no useful signal).
#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub(in crate::mcp) struct LabelProviderMissingSummary {
    no_discogs: usize,
    no_musicbrainz: usize,
    no_bandcamp: usize,
}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub(in crate::mcp) struct BackfillLabelsSummary {
    total_scanned: usize,
    filled: usize,
    already_labeled: usize,
    conflicts: usize,
    no_enrichment: usize,
    no_enrichment_by_provider: LabelProviderMissingSummary,
}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub(in crate::mcp) struct LabelResearchQueue {
    total_unlabeled: usize,
    top_artists: Vec<serde_json::Value>,
}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub(in crate::mcp) struct BackfillLabelsOutput {
    summary: BackfillLabelsSummary,
    staged: usize,
    total_pending: usize,
    dry_run: bool,
    conflicts: Vec<serde_json::Value>,
    conflict_page: OffsetPage,
    #[serde(skip_serializing_if = "Option::is_none")]
    conflicts_truncated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    auto_enriched: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    auto_enriched_by_provider: Option<LabelAutoEnrichedByProvider>,
    #[serde(skip_serializing_if = "Option::is_none")]
    auto_enrichment: Option<MetadataAutoEnrichmentReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    research_queue: Option<LabelResearchQueue>,
}

/// Successful provider matches returned during this invocation. The additive
/// total remains `auto_enriched` for wire compatibility.
#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub(in crate::mcp) struct LabelAutoEnrichedByProvider {
    musicbrainz: usize,
    bandcamp: usize,
}

#[cfg(test)]
impl LabelAutoEnrichedByProvider {
    fn total(&self) -> usize {
        self.musicbrainz + self.bandcamp
    }
}

pub(in crate::mcp) async fn handle_backfill_labels(
    server: &ReklawdboxServer,
    params: BackfillLabelsParams,
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
        let scan = scan_labels(&store_conn, &tracks);
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
            "auto_enrich: fetching labels for {total} uncached provider keys"
        );

        // The writer opens a separate connection after the store is initialized.
        {
            let _conn = server.cache_store_conn()?;
        }
        let (writer_provider, writer_request) = bandcamp_requests
            .first()
            .map(|request| (MetadataEnrichmentProvider::Bandcamp, request))
            .or_else(|| {
                musicbrainz_requests
                    .first()
                    .map(|request| (MetadataEnrichmentProvider::MusicBrainz, request))
            })
            .expect("label auto-enrichment requests should be non-empty");
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
        scan = scan_labels(&store_conn, &tracks);
        drop(store_conn);
    }
    let auto_enriched_by_provider = LabelAutoEnrichedByProvider {
        musicbrainz: auto_enrichment.matched_by(MetadataEnrichmentProvider::MusicBrainz),
        bandcamp: auto_enrichment.matched_by(MetadataEnrichmentProvider::Bandcamp),
    };
    let auto_enriched = auto_enrichment.matched;

    // Collect before to_stage is moved by stage().
    let staged_label_ids: std::collections::HashSet<String> = scan
        .to_stage
        .iter()
        .filter(|c| c.label.is_some())
        .map(|c| c.track_id.clone())
        .collect();

    let (staged_count, pending) =
        stage_suggestions(&server.context.mutation.changes, scan.to_stage, dry_run);

    let max_conflicts = params.max_conflicts.unwrap_or(50);
    let conflict_offset = params.conflict_offset.unwrap_or(0);
    let total_conflicts = scan.conflicts.len();
    let (conflict_range, conflict_page) =
        offset_page_bounds(total_conflicts, conflict_offset, max_conflicts);
    let conflicts = scan.conflicts[conflict_range].to_vec();
    let conflicts_truncated = conflict_page.has_more.then_some(true);

    // Excludes tracks just staged — they're unlabeled in the read-only DB
    // but will resolve on export, so they don't need research.
    let (unlabeled_count, research_queue) = {
        let rb_conn = server.rekordbox_conn()?;
        let search_params = db::SearchParams {
            has_label: Some(false),
            exclude_samples: true,
            ..Default::default()
        };
        let unlabeled = db::search_tracks_unbounded(&rb_conn, &search_params).map_err(db_error)?;
        let unlabeled: Vec<_> = unlabeled
            .into_iter()
            .filter(|t| !staged_label_ids.contains(&t.id))
            .collect();
        let count = unlabeled.len();

        let research_queue = if count > 0 {
            // Group by artist, sorted by count descending
            let mut by_artist: std::collections::HashMap<&str, usize> =
                std::collections::HashMap::new();
            for t in &unlabeled {
                *by_artist.entry(&t.artist).or_insert(0) += 1;
            }
            let mut artist_counts: Vec<_> = by_artist.into_iter().collect();
            artist_counts.sort_by(|left, right| {
                right.1.cmp(&left.1).then_with(|| {
                    normalize::normalize_for_matching(left.0)
                        .cmp(&normalize::normalize_for_matching(right.0))
                })
            });
            let top_artists: Vec<_> = artist_counts
                .iter()
                .take(20)
                .map(|(artist, count)| serde_json::json!({"artist": artist, "count": count}))
                .collect();

            Some(LabelResearchQueue {
                total_unlabeled: count,
                top_artists,
            })
        } else {
            None
        };
        (count, research_queue)
    };

    // Always write (including 0) so the gate clears after a successful re-run.
    if !dry_run {
        server
            .context
            .mutation
            .label_research_gate
            .store(unlabeled_count as u32, std::sync::atomic::Ordering::Relaxed);
    }

    ok_structured_json(BackfillLabelsOutput {
        summary: BackfillLabelsSummary {
            total_scanned: tracks.len(),
            filled: scan.filled,
            already_labeled: scan.already_labeled,
            conflicts: total_conflicts,
            no_enrichment: scan.no_enrichment,
            no_enrichment_by_provider: LabelProviderMissingSummary {
                no_discogs: scan.no_discogs,
                no_musicbrainz: scan.no_musicbrainz,
                no_bandcamp: scan.no_bandcamp,
            },
        },
        staged: staged_count,
        total_pending: pending,
        dry_run,
        conflicts,
        conflict_page,
        conflicts_truncated,
        auto_enriched: auto_enrich.then_some(auto_enriched),
        auto_enriched_by_provider: auto_enrich.then_some(auto_enriched_by_provider),
        auto_enrichment: auto_enrich.then_some(auto_enrichment),
        research_queue,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::metadata::backfill::{
        BackfillLabelsScanResult, normalize_label, sort_label_conflicts,
    };

    // ── normalize_label ──────────────────────────────────────────────

    #[test]
    fn normalize_label_filters_not_on_label_exact() {
        assert_eq!(normalize_label("Not On Label"), None);
    }

    #[test]
    fn normalize_label_filters_not_on_label_with_suffix() {
        // Discogs uses "Not On Label (Artist Self-released)" and similar variants
        assert_eq!(normalize_label("Not On Label (Artist Self-released)"), None);
        assert_eq!(normalize_label("Not On Label (Some Other Variant)"), None);
    }

    #[test]
    fn normalize_label_passes_regular_labels() {
        assert_eq!(
            normalize_label("Warp Records"),
            Some("Warp Records".to_string())
        );
        assert_eq!(normalize_label("Planet Mu"), Some("Planet Mu".to_string()));
        assert_eq!(normalize_label("Hyperdub"), Some("Hyperdub".to_string()));
    }

    #[test]
    fn normalize_label_passes_empty_string() {
        // Empty labels are not "Not On Label" — caller filters empties separately
        assert_eq!(normalize_label(""), Some(String::new()));
    }

    #[test]
    fn normalize_label_is_case_sensitive() {
        // "not on label" (lowercase) is not filtered — Discogs always uses title case
        assert_eq!(
            normalize_label("not on label"),
            Some("not on label".to_string())
        );
        assert_eq!(
            normalize_label("NOT ON LABEL"),
            Some("NOT ON LABEL".to_string())
        );
    }

    #[test]
    fn normalize_label_preserves_similar_names() {
        // Labels that happen to contain "Not On Label" as a substring but don't
        // start with it should pass through
        assert_eq!(
            normalize_label("Definitely Not On Label Records"),
            Some("Definitely Not On Label Records".to_string())
        );
    }

    // ── BackfillLabelsScanResult default ─────────────────────────────

    #[test]
    fn scan_result_default_is_zeroed() {
        let r = BackfillLabelsScanResult::default();
        assert_eq!(r.filled, 0);
        assert_eq!(r.already_labeled, 0);
        assert!(r.conflicts.is_empty());
        assert_eq!(r.no_enrichment, 0);
        assert_eq!(r.no_discogs, 0);
        assert_eq!(r.no_musicbrainz, 0);
        assert_eq!(r.no_bandcamp, 0);
        assert!(r.to_stage.is_empty());
        assert!(r.uncached_musicbrainz.is_empty());
        assert!(r.uncached_bandcamp.is_empty());
    }

    #[test]
    fn label_auto_enrichment_provider_totals_are_additive() {
        let counts = LabelAutoEnrichedByProvider {
            musicbrainz: 3,
            bandcamp: 2,
        };
        assert_eq!(counts.total(), 5);
    }

    #[test]
    fn backfill_labels_conflict_page_is_stable_and_non_overlapping() {
        let mut conflicts = vec![
            serde_json::json!({"track_id": "t3", "artist": "Zulu", "title": "One"}),
            serde_json::json!({"track_id": "t2", "artist": "Alpha", "title": "Same"}),
            serde_json::json!({"track_id": "t1", "artist": "Alpha", "title": "Same"}),
        ];
        sort_label_conflicts(&mut conflicts);
        let ids: Vec<_> = conflicts
            .iter()
            .map(|conflict| conflict["track_id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["t1", "t2", "t3"]);

        let (first_range, first) = offset_page_bounds(conflicts.len(), 0, 2);
        let (second_range, second) = offset_page_bounds(conflicts.len(), 2, 2);
        assert_eq!(first_range, 0..2);
        assert_eq!(second_range, 2..3);
        assert_eq!(
            first,
            OffsetPage {
                total: 3,
                returned: 2,
                offset: 0,
                next_offset: Some(2),
                has_more: true,
            }
        );
        assert_eq!(
            second,
            OffsetPage {
                total: 3,
                returned: 1,
                offset: 2,
                next_offset: None,
                has_more: false,
            }
        );
    }

    #[test]
    fn backfill_labels_conflict_page_zero_and_beyond_end_are_terminal() {
        let (zero_range, zero) = offset_page_bounds(3, 1, 0);
        assert_eq!(zero_range, 1..1);
        assert_eq!(zero.returned, 0);
        assert_eq!(zero.next_offset, None);
        assert!(!zero.has_more);

        let (beyond_range, beyond) = offset_page_bounds(3, usize::MAX, 10);
        assert_eq!(beyond_range, 3..3);
        assert_eq!(beyond.returned, 0);
        assert_eq!(beyond.next_offset, None);
        assert!(!beyond.has_more);
    }
}
