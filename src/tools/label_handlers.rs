use rmcp::ErrorData as McpError;
use rmcp::model::{CallToolResult, Content};
use schemars::JsonSchema;
use serde::Deserialize;

use super::*;
use crate::db;
use crate::normalize;
use crate::store;
use crate::types::TrackChange;

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct BackfillLabelsParams {
    #[schemars(description = "Preview changes without staging (default false)")]
    pub dry_run: Option<bool>,
    #[schemars(
        description = "Automatically enrich uncached tracks via Bandcamp before backfilling (default false). Fetches Bandcamp data for tracks missing from all enrichment caches, then re-scans."
    )]
    pub auto_enrich: Option<bool>,
    #[schemars(
        description = "Max conflict entries to include in the response (default 50). Use search_tracks to page through remaining conflicts."
    )]
    pub max_conflicts: Option<usize>,
}

/// Filters out Discogs "Not On Label" entries (self-released, no useful signal).
fn normalize_label(label: &str) -> Option<String> {
    if label.starts_with("Not On Label") {
        return None;
    }
    Some(label.to_string())
}

fn extract_label(entry: Option<&store::EnrichmentCacheEntry>) -> Option<String> {
    classify_handler::parse_response_json(entry)
        .as_ref()
        .and_then(|v| v.get("label"))
        .and_then(|v| v.as_str())
        .filter(|l| !l.is_empty())
        .and_then(normalize_label)
}

#[derive(Default)]
struct BackfillLabelsScanResult {
    filled: usize,
    already_labeled: usize,
    conflicts: Vec<serde_json::Value>,
    no_enrichment: usize,
    no_discogs: usize,
    no_musicbrainz: usize,
    no_bandcamp: usize,
    no_beatport: usize,
    to_stage: Vec<TrackChange>,
    /// Tracks that had no Bandcamp cache and got no label from any source.
    uncached_bandcamp: Vec<(String, String, String, String)>, // (norm_artist, norm_title, raw_artist, raw_title)
}

fn scan_labels(
    store_conn: &rusqlite::Connection,
    tracks: &[crate::types::Track],
) -> BackfillLabelsScanResult {
    let mut result = BackfillLabelsScanResult::default();

    // Pre-compute normalized keys for all tracks.
    let norm_keys: Vec<(String, String, Option<String>)> = tracks
        .iter()
        .map(|t| {
            let a = normalize::normalize_for_matching(&t.artist);
            let ti = normalize::normalize_for_matching(&t.title);
            let al = normalize::normalize_for_matching(&t.album);
            (a, ti, (!al.is_empty()).then_some(al))
        })
        .collect();

    // Build batch keys for all 4 providers × all tracks.
    let mut enrich_keys: Vec<(&str, &str, &str, &str)> = Vec::with_capacity(tracks.len() * 4);
    for (a, t, al) in &norm_keys {
        let album = al.as_deref().unwrap_or("");
        enrich_keys.push(("discogs", a, t, album));
        enrich_keys.push(("musicbrainz", a, t, ""));
        enrich_keys.push(("bandcamp", a, t, ""));
        enrich_keys.push(("beatport", a, t, ""));
    }

    // Single batch load — replaces 4N individual queries.
    let cache_map = store::batch_get_enrichment(store_conn, &enrich_keys).unwrap_or_else(|e| {
        tracing::warn!("batch enrichment load failed: {e}");
        std::collections::HashMap::new()
    });

    for (track, (norm_artist, norm_title, norm_album)) in tracks.iter().zip(&norm_keys) {
        let album = norm_album.as_deref().unwrap_or("");

        let discogs_key = (
            "discogs".to_string(),
            norm_artist.clone(),
            norm_title.clone(),
            album.to_string(),
        );
        let mb_key = (
            "musicbrainz".to_string(),
            norm_artist.clone(),
            norm_title.clone(),
            String::new(),
        );
        let bc_key = (
            "bandcamp".to_string(),
            norm_artist.clone(),
            norm_title.clone(),
            String::new(),
        );
        let bp_key = (
            "beatport".to_string(),
            norm_artist.clone(),
            norm_title.clone(),
            String::new(),
        );

        let discogs_entry = cache_map.get(&discogs_key);
        let mb_entry = cache_map.get(&mb_key);
        let bc_entry = cache_map.get(&bc_key);
        let bp_entry = cache_map.get(&bp_key);

        let discogs_label = extract_label(discogs_entry);
        let mb_label = extract_label(mb_entry);
        let bc_label = extract_label(bc_entry);
        let bp_label = extract_label(bp_entry);

        let enrichment_label = discogs_label.or(mb_label).or(bc_label).or(bp_label);

        let Some(enrich_label) = enrichment_label else {
            result.no_enrichment += 1;
            if discogs_entry.is_none() {
                result.no_discogs += 1;
            }
            if mb_entry.is_none() {
                result.no_musicbrainz += 1;
            }
            if bc_entry.is_none() {
                result.no_bandcamp += 1;
                result.uncached_bandcamp.push((
                    norm_artist.clone(),
                    norm_title.clone(),
                    track.artist.clone(),
                    track.title.clone(),
                ));
            }
            if bp_entry.is_none() {
                result.no_beatport += 1;
            }
            continue;
        };

        if track.label.is_empty() {
            result.filled += 1;
            result.to_stage.push(TrackChange {
                track_id: track.id.clone(),
                genre: None,
                comments: None,
                rating: None,
                color: None,
                label: Some(enrich_label.clone()),
                year: None,
                album: None,
            });
        } else if track.label.eq_ignore_ascii_case(&enrich_label) {
            result.already_labeled += 1;
        } else {
            result.conflicts.push(serde_json::json!({
                "track_id": track.id,
                "artist": track.artist,
                "title": track.title,
                "current_label": track.label,
                "enrichment_label": enrich_label,
            }));
        }
    }

    result
}

pub(super) async fn handle_backfill_labels(
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

    let staged_count = if !dry_run && !scan.to_stage.is_empty() {
        let (staged, _) = server.state.changes.stage(scan.to_stage);
        staged
    } else {
        0
    };

    let pending = server.state.changes.pending_ids().len();

    let max_conflicts = params.max_conflicts.unwrap_or(50);
    let total_conflicts = scan.conflicts.len();
    let conflicts_truncated = total_conflicts > max_conflicts;
    scan.conflicts.truncate(max_conflicts);

    let mut result = serde_json::json!({
        "summary": {
            "total_scanned": tracks.len(),
            "filled": scan.filled,
            "already_labeled": scan.already_labeled,
            "conflicts": total_conflicts,
            "no_enrichment": scan.no_enrichment,
            "no_enrichment_by_provider": {
                "no_discogs": scan.no_discogs,
                "no_musicbrainz": scan.no_musicbrainz,
                "no_bandcamp": scan.no_bandcamp,
                "no_beatport": scan.no_beatport,
            },
        },
        "staged": staged_count,
        "total_pending": pending,
        "dry_run": dry_run,
        "conflicts": scan.conflicts,
    });
    if conflicts_truncated {
        result["conflicts_truncated"] = serde_json::json!(true);
    }

    if auto_enrich {
        result.as_object_mut().unwrap().insert(
            "auto_enriched".to_string(),
            serde_json::json!(auto_enriched),
        );
    }

    // Excludes tracks just staged — they're unlabeled in the read-only DB
    // but will resolve on export, so they don't need research.
    let unlabeled_count = {
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

        if count > 0 {
            // Group by artist, sorted by count descending
            let mut by_artist: std::collections::HashMap<&str, usize> =
                std::collections::HashMap::new();
            for t in &unlabeled {
                *by_artist.entry(&t.artist).or_insert(0) += 1;
            }
            let mut artist_counts: Vec<_> = by_artist.into_iter().collect();
            artist_counts.sort_by(|a, b| b.1.cmp(&a.1));
            let top_artists: Vec<_> = artist_counts
                .iter()
                .take(20)
                .map(|(artist, count)| serde_json::json!({"artist": artist, "count": count}))
                .collect();

            result.as_object_mut().unwrap().insert(
                "research_queue".to_string(),
                serde_json::json!({
                    "total_unlabeled": count,
                    "top_artists": top_artists,
                }),
            );
        }
        count
    };

    // Always write (including 0) so the gate clears after a successful re-run.
    if !dry_run {
        server
            .state
            .label_research_gate
            .store(unlabeled_count as u32, std::sync::atomic::Ordering::Relaxed);
    }

    let json = serde_json::to_string(&result).map_err(|e| mcp_internal_error(format!("{e}")))?;
    Ok(CallToolResult::success(vec![Content::text(json)]))
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
