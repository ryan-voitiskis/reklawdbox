use rmcp::ErrorData as McpError;
use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::adapters::rekordbox as db;
use crate::adapters::state as store;
use crate::application::metadata::backfill::{scan_labels, stage_suggestions};
use crate::domain::metadata as normalize;
use crate::mcp::{
    OffsetPage, ReklawdboxServer, db_error, lookup_bandcamp_remote, mcp_internal_error,
    offset_page_bounds, ok_structured_json,
};

#[derive(Debug, Deserialize, JsonSchema)]
pub(in crate::mcp) struct BackfillLabelsParams {
    #[schemars(description = "Preview changes without staging (default false)")]
    pub dry_run: Option<bool>,
    #[schemars(
        description = "Automatically enrich uncached tracks via Bandcamp before backfilling (default false). Fetches Bandcamp data for tracks missing from all enrichment caches, then re-scans."
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
    no_beatport: usize,
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
    research_queue: Option<LabelResearchQueue>,
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

    let mut auto_enriched = 0usize;

    if auto_enrich && !scan.uncached_bandcamp.is_empty() {
        let to_enrich: Vec<_> = std::mem::take(&mut scan.uncached_bandcamp);
        let total = to_enrich.len();
        tracing::info!(
            count = total,
            "auto_enrich: fetching Bandcamp for uncached label tracks"
        );

        let concurrency = 4usize;
        let store_path = server.cache_store_path();

        // Ensure DB exists and is migrated before spawning the writer.
        {
            let _conn = server.cache_store_conn()?;
        }

        let (cache_tx, mut cache_rx) =
            tokio::sync::mpsc::channel::<(String, String, Option<String>, Option<String>)>(
                concurrency * 4,
            );

        let writer_store_path = store_path.clone();
        let writer_handle = tokio::task::spawn_blocking(move || {
            let conn = match store::open(&writer_store_path) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("label backfill cache writer: failed to open store: {e}");
                    return;
                }
            };
            while let Some((norm_artist, norm_title, match_quality, response_json)) =
                cache_rx.blocking_recv()
            {
                if let Err(e) = store::set_enrichment(
                    &conn,
                    "bandcamp",
                    &norm_artist,
                    &norm_title,
                    None,
                    match_quality.as_deref(),
                    response_json.as_deref(),
                ) {
                    tracing::warn!(
                        artist = norm_artist.as_str(),
                        title = norm_title.as_str(),
                        "label backfill cache write failed: {e}"
                    );
                }
            }
        });

        let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(concurrency));
        let mut handles = Vec::with_capacity(to_enrich.len());

        for (norm_artist, norm_title, raw_artist, raw_title) in to_enrich {
            let permit = sem
                .clone()
                .acquire_owned()
                .await
                .map_err(|e| mcp_internal_error(format!("Semaphore error: {e}")))?;
            let server = server.clone();
            let cache_tx = cache_tx.clone();

            handles.push(tokio::spawn(async move {
                let result = lookup_bandcamp_remote(&server, &raw_artist, &raw_title).await;
                let hit = match result {
                    Ok(ref r) => {
                        let (quality, json) = match r {
                            Some(br) => {
                                let json_str = match serde_json::to_string(br) {
                                    Ok(j) => j,
                                    Err(e) => {
                                        tracing::warn!(
                                            artist = norm_artist.as_str(),
                                            "serialization failed: {e}"
                                        );
                                        drop(permit);
                                        return 1usize;
                                    }
                                };
                                let q = if br.score >= 90 {
                                    "exact".to_string()
                                } else {
                                    "fuzzy".to_string()
                                };
                                (Some(q), Some(json_str))
                            }
                            None => (Some("none".to_string()), None),
                        };
                        if let Err(e) = cache_tx
                            .send((norm_artist, norm_title, quality, json))
                            .await
                        {
                            tracing::warn!("label backfill cache channel send failed: {e}");
                        }
                        r.is_some() as usize
                    }
                    Err(e) => {
                        tracing::warn!(
                            artist = raw_artist.as_str(),
                            "Bandcamp auto-enrich failed: {e}"
                        );
                        0
                    }
                };
                drop(permit);
                hit
            }));
        }

        for handle in handles {
            match handle.await {
                Ok(hit) => auto_enriched += hit,
                Err(e) => {
                    tracing::warn!("label backfill task panicked: {e}");
                }
            }
        }

        drop(cache_tx);
        if let Err(e) = writer_handle.await {
            tracing::warn!("label backfill cache writer task failed: {e}");
        }

        let store_conn = server.cache_store_conn()?;
        scan = scan_labels(&store_conn, &tracks);
        drop(store_conn);
    }

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
                no_beatport: scan.no_beatport,
            },
        },
        staged: staged_count,
        total_pending: pending,
        dry_run,
        conflicts,
        conflict_page,
        conflicts_truncated,
        auto_enriched: auto_enrich.then_some(auto_enriched),
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
        assert_eq!(r.no_beatport, 0);
        assert!(r.to_stage.is_empty());
        assert!(r.uncached_bandcamp.is_empty());
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
