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
}

/// Filter out Discogs "Not On Label" entries (self-released, no useful signal).
/// Everything else passes through unchanged.
fn normalize_label(label: &str) -> Option<String> {
    if label.starts_with("Not On Label") {
        return None;
    }
    Some(label.to_string())
}

/// Cache a lookup result (hit or miss). Returns 1 if the result had data, 0 if miss.
fn cache_lookup_result<T: serde::Serialize + HasScore>(
    server: &ReklawdboxServer,
    provider: &str,
    norm_artist: &str,
    norm_title: &str,
    result: Option<&T>,
) -> Result<usize, McpError> {
    let store_conn = server.cache_store_conn()?;
    match result {
        Some(r) => {
            let json = serde_json::to_string(r).ok();
            let quality = if r.score() >= 90 { "exact" } else { "fuzzy" };
            let _ = store::set_enrichment(
                &store_conn,
                provider,
                norm_artist,
                norm_title,
                None,
                Some(quality),
                json.as_deref(),
            );
            Ok(1)
        }
        None => {
            let _ = store::set_enrichment(
                &store_conn,
                provider,
                norm_artist,
                norm_title,
                None,
                Some("none"),
                None,
            );
            Ok(0)
        }
    }
}

trait HasScore {
    fn score(&self) -> i32;
}

impl HasScore for crate::bandcamp::BandcampResult {
    fn score(&self) -> i32 {
        self.score
    }
}

/// Try to extract a label from the enrichment cache for a single provider.
fn label_from_cache(
    store_conn: &rusqlite::Connection,
    provider: &str,
    norm_artist: &str,
    norm_title: &str,
    norm_album: Option<&str>,
) -> (bool, Option<String>) {
    let cache = store::get_enrichment(store_conn, provider, norm_artist, norm_title, norm_album)
        .unwrap_or_else(|e| {
            tracing::warn!(
                provider,
                artist = norm_artist,
                title = norm_title,
                "cache lookup failed: {e}"
            );
            None
        });
    let has_cache = cache.is_some();
    let label = classify_handler::parse_response_json(cache.as_ref())
        .as_ref()
        .and_then(|v| v.get("label"))
        .and_then(|v| v.as_str())
        .filter(|l| !l.is_empty())
        .and_then(normalize_label);
    (has_cache, label)
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
    to_stage: Vec<TrackChange>,
    /// Tracks that had no Bandcamp cache and got no label from any source.
    uncached_bandcamp: Vec<(String, String, String, String)>, // (norm_artist, norm_title, raw_artist, raw_title)
}

fn scan_labels(
    store_conn: &rusqlite::Connection,
    tracks: &[crate::types::Track],
) -> BackfillLabelsScanResult {
    let mut result = BackfillLabelsScanResult::default();

    for track in tracks {
        let norm_artist = normalize::normalize_for_matching(&track.artist);
        let norm_title = normalize::normalize_for_matching(&track.title);
        let norm_album = normalize::normalize_for_matching(&track.album);
        let norm_album = (!norm_album.is_empty()).then_some(norm_album);

        let (has_discogs, discogs_label) = label_from_cache(
            store_conn,
            "discogs",
            &norm_artist,
            &norm_title,
            norm_album.as_deref(),
        );
        let (has_mb, mb_label) =
            label_from_cache(store_conn, "musicbrainz", &norm_artist, &norm_title, None);
        let (has_bc, bc_label) =
            label_from_cache(store_conn, "bandcamp", &norm_artist, &norm_title, None);

        let enrichment_label = discogs_label.or(mb_label).or(bc_label);

        let Some(enrich_label) = enrichment_label else {
            result.no_enrichment += 1;
            if !has_discogs {
                result.no_discogs += 1;
            }
            if !has_mb {
                result.no_musicbrainz += 1;
            }
            if !has_bc {
                result.no_bandcamp += 1;
                result.uncached_bandcamp.push((
                    norm_artist,
                    norm_title,
                    track.artist.clone(),
                    track.title.clone(),
                ));
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
        let tracks = db::search_tracks_unbounded(&rb_conn, &search_params)
            .map_err(|e| mcp_internal_error(format!("DB error: {e}")))?;
        drop(rb_conn);

        let store_conn = server.cache_store_conn()?;
        let scan = scan_labels(&store_conn, &tracks);
        drop(store_conn);
        (tracks, scan)
    };

    let mut auto_enriched = 0usize;

    // Auto-enrich: fetch Bandcamp for tracks with no cached Bandcamp data.
    if auto_enrich && !scan.uncached_bandcamp.is_empty() {
        let to_enrich: Vec<_> = std::mem::take(&mut scan.uncached_bandcamp);
        let total = to_enrich.len();
        tracing::info!(
            count = total,
            "auto_enrich: fetching Bandcamp for uncached label tracks"
        );

        for (norm_artist, norm_title, raw_artist, raw_title) in &to_enrich {
            match lookup_bandcamp_remote(server, raw_artist, raw_title).await {
                Ok(result) => {
                    auto_enriched += cache_lookup_result(
                        server,
                        "bandcamp",
                        norm_artist,
                        norm_title,
                        result.as_ref(),
                    )?;
                }
                Err(e) => {
                    tracing::warn!(
                        artist = raw_artist.as_str(),
                        "Bandcamp auto-enrich failed: {e}"
                    );
                }
            }
        }

        // Second pass: re-scan with updated cache.
        let store_conn = server.cache_store_conn()?;
        scan = scan_labels(&store_conn, &tracks);
        drop(store_conn);
    }

    let staged_count = if !dry_run && !scan.to_stage.is_empty() {
        let (staged, _) = server.state.changes.stage(scan.to_stage);
        staged
    } else {
        0
    };

    let pending = server.state.changes.pending_ids().len();

    let mut result = serde_json::json!({
        "summary": {
            "total_scanned": tracks.len(),
            "filled": scan.filled,
            "already_labeled": scan.already_labeled,
            "conflicts": scan.conflicts.len(),
            "no_enrichment": scan.no_enrichment,
            "no_enrichment_by_provider": {
                "no_discogs": scan.no_discogs,
                "no_musicbrainz": scan.no_musicbrainz,
                "no_bandcamp": scan.no_bandcamp,
            },
        },
        "staged": staged_count,
        "total_pending": pending,
        "dry_run": dry_run,
        "conflicts": scan.conflicts,
    });

    if auto_enrich {
        result.as_object_mut().unwrap().insert(
            "auto_enriched".to_string(),
            serde_json::json!(auto_enriched),
        );
    }

    let json =
        serde_json::to_string_pretty(&result).map_err(|e| mcp_internal_error(format!("{e}")))?;
    Ok(CallToolResult::success(vec![Content::text(json)]))
}
