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
}

/// Filter out Discogs "Not On Label" entries (self-released, no useful signal).
/// Everything else passes through unchanged.
fn normalize_label(label: &str) -> Option<String> {
    if label.starts_with("Not On Label") {
        return None;
    }
    Some(label.to_string())
}

pub(super) fn handle_backfill_labels(
    server: &ReklawdboxServer,
    params: BackfillLabelsParams,
) -> Result<CallToolResult, McpError> {
    let dry_run = params.dry_run.unwrap_or(false);

    let rb_conn = server.rekordbox_conn()?;
    let search_params = db::SearchParams {
        query: None,
        artist: None,
        genre: None,
        rating_min: None,
        bpm_min: None,
        bpm_max: None,
        key: None,
        playlist: None,
        has_genre: None,
        has_label: None,
        label: None,
        path: None,
        path_prefix: None,
        added_after: None,
        added_before: None,
        exclude_samples: true,
        limit: None,
        offset: None,
    };
    let tracks = db::search_tracks_unbounded(&rb_conn, &search_params)
        .map_err(|e| mcp_internal_error(format!("DB error: {e}")))?;
    drop(rb_conn);

    let store_conn = server.cache_store_conn()?;

    let mut filled = 0usize;
    let mut already_labeled = 0usize;
    let mut conflicts = Vec::new();
    let mut no_enrichment = 0usize;
    let mut to_stage = Vec::new();

    for track in &tracks {
        let norm_artist = normalize::normalize_for_matching(&track.artist);
        let norm_title = normalize::normalize_for_matching(&track.title);
        let norm_album = normalize::normalize_for_matching(&track.album);
        let norm_album = (!norm_album.is_empty()).then_some(norm_album);

        let discogs_cache = match store::get_enrichment(
            &store_conn,
            "discogs",
            &norm_artist,
            &norm_title,
            norm_album.as_deref(),
        ) {
            Ok(cache) => cache,
            Err(e) => {
                tracing::warn!(
                    track_id = track.id.as_str(),
                    "Enrichment cache lookup failed: {e}"
                );
                no_enrichment += 1;
                continue;
            }
        };

        let discogs_val = classify_handler::parse_response_json(discogs_cache.as_ref());
        let enrichment_label = discogs_val
            .as_ref()
            .and_then(|v| v.get("label"))
            .and_then(|v| v.as_str())
            .filter(|l| !l.is_empty())
            .and_then(normalize_label);

        // Fall back to MusicBrainz if Discogs has no label
        let enrichment_label = enrichment_label.or_else(|| {
            let mb_cache = match store::get_enrichment(
                &store_conn,
                "musicbrainz",
                &norm_artist,
                &norm_title,
                None,
            ) {
                Ok(cache) => cache,
                Err(e) => {
                    tracing::warn!(
                        track_id = track.id.as_str(),
                        "MusicBrainz enrichment cache lookup failed: {e}"
                    );
                    return None;
                }
            };
            let mb_val = super::classify_handler::parse_response_json(mb_cache.as_ref());
            mb_val
                .as_ref()
                .and_then(|v| v.get("label"))
                .and_then(|v| v.as_str())
                .filter(|l| !l.is_empty())
                .and_then(normalize_label)
        });

        let Some(enrich_label) = enrichment_label else {
            no_enrichment += 1;
            continue;
        };

        if track.label.is_empty() {
            filled += 1;
            to_stage.push(TrackChange {
                track_id: track.id.clone(),
                genre: None,
                comments: None,
                rating: None,
                color: None,
                label: Some(enrich_label.clone()),
                year: None,
            });
        } else if track.label.eq_ignore_ascii_case(&enrich_label) {
            already_labeled += 1;
        } else {
            conflicts.push(serde_json::json!({
                "track_id": track.id,
                "artist": track.artist,
                "title": track.title,
                "current_label": track.label,
                "enrichment_label": enrich_label,
            }));
        }
    }

    drop(store_conn);

    let staged_count = if !dry_run && !to_stage.is_empty() {
        let (staged, _) = server.state.changes.stage(to_stage);
        staged
    } else {
        0
    };

    let pending = server.state.changes.pending_ids().len();

    let result = serde_json::json!({
        "summary": {
            "total_scanned": tracks.len(),
            "filled": filled,
            "already_labeled": already_labeled,
            "conflicts": conflicts.len(),
            "no_enrichment": no_enrichment,
        },
        "staged": staged_count,
        "total_pending": pending,
        "dry_run": dry_run,
        "conflicts": conflicts,
    });

    let json =
        serde_json::to_string_pretty(&result).map_err(|e| mcp_internal_error(format!("{e}")))?;
    Ok(CallToolResult::success(vec![Content::text(json)]))
}
