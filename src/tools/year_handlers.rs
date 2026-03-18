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
pub(super) struct BackfillYearsParams {
    #[schemars(description = "Preview changes without staging (default false)")]
    pub dry_run: Option<bool>,
}

fn parse_discogs_year(s: &str) -> Option<i32> {
    let trimmed = s.trim();
    let year: i32 = trimmed.parse().ok()?;
    (1900..=2099).contains(&year).then_some(year)
}

pub(super) fn handle_backfill_years(
    server: &ReklawdboxServer,
    params: BackfillYearsParams,
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
    let mut already_set = 0usize;
    let mut conflicts = Vec::new();
    let mut no_enrichment = 0usize;
    let mut no_year_in_enrichment = 0usize;
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
        let enrichment_year = discogs_val
            .as_ref()
            .and_then(|v| v.get("year"))
            .and_then(|v| match v {
                serde_json::Value::Number(n) => n.as_i64().map(|n| n.to_string()),
                serde_json::Value::String(s) => Some(s.clone()),
                _ => None,
            })
            .and_then(|s| parse_discogs_year(&s));

        let Some(enrich_year) = enrichment_year else {
            if discogs_val.is_some() {
                no_year_in_enrichment += 1;
            } else {
                no_enrichment += 1;
            }
            continue;
        };

        if track.year == 0 {
            filled += 1;
            to_stage.push(TrackChange {
                track_id: track.id.clone(),
                genre: None,
                comments: None,
                rating: None,
                color: None,
                label: None,
                year: Some(enrich_year),
            });
        } else if track.year == enrich_year {
            already_set += 1;
        } else {
            conflicts.push(serde_json::json!({
                "track_id": track.id,
                "artist": track.artist,
                "title": track.title,
                "current_year": track.year,
                "enrichment_year": enrich_year,
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
            "already_set": already_set,
            "conflicts": conflicts.len(),
            "no_year_in_enrichment": no_year_in_enrichment,
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
