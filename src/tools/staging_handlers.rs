use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::MutexGuard;

use rmcp::ErrorData as McpError;
use rmcp::model::{CallToolResult, Content};
use rusqlite::Connection;

use super::*;
use crate::changes::ChangeManager;
use crate::color;
use crate::db;
use crate::genre;
use crate::types::TrackChange;
use crate::xml;

pub(super) fn handle_update_tracks(
    changes: &ChangeManager,
    params: UpdateTracksParams,
) -> Result<CallToolResult, McpError> {
    for c in &params.changes {
        if c.track_id.trim().is_empty() {
            return Err(McpError::invalid_params("track_id must not be empty", None));
        }
        if let Some(r) = c.rating
            && (r == 0 || r > 5)
        {
            return Err(McpError::invalid_params(
                format!("rating must be 1-5, got {r}"),
                None,
            ));
        }
        if let Some(y) = c.year
            && !(1900..=2099).contains(&y)
        {
            return Err(McpError::invalid_params(
                format!("year must be 1900-2099, got {y}"),
                None,
            ));
        }
        if let Some(ref col) = c.color
            && !color::is_valid_color(col)
        {
            let valid: Vec<&str> = color::COLORS.iter().map(|(n, _)| *n).collect();
            return Err(McpError::invalid_params(
                format!(
                    "unknown color '{}'. Valid colors: {}",
                    col,
                    valid.join(", ")
                ),
                None,
            ));
        }
    }

    let mut warnings: Vec<String> = Vec::new();
    for c in &params.changes {
        if let Some(ref g) = c.genre
            && !genre::is_known_genre(g)
        {
            warnings.push(format!("'{g}' is not in the genre taxonomy"));
        }
    }

    let track_changes: Vec<TrackChange> = params
        .changes
        .into_iter()
        .map(|c| TrackChange {
            track_id: c.track_id,
            genre: c.genre,
            comments: c.comments,
            rating: c.rating,
            color: c.color.map(|col| {
                color::canonical_color_name(&col)
                    .map(String::from)
                    .unwrap_or(col)
            }),
            label: c.label,
            year: c.year,
            album: c.album,
        })
        .collect();

    let (staged, total) = changes.stage(track_changes);
    let mut result = serde_json::json!({
        "staged": staged,
        "total_pending": total,
    });
    if !warnings.is_empty() {
        result["warnings"] = serde_json::json!(warnings);
    }
    ok_json(&result)
}

pub(super) fn handle_suggest_normalizations(
    conn: MutexGuard<'_, Connection>,
    changes: &ChangeManager,
    params: SuggestNormalizationsParams,
) -> Result<CallToolResult, McpError> {
    let min_count = params.min_genre_count.unwrap_or(1);
    let stage_aliases = params.stage_aliases.unwrap_or(false);

    let stats = db::get_library_stats(&conn).map_err(db_error)?;

    // Grouped alias: (current_genre, canonical) → [track_ids]
    let mut alias_groups: Vec<(String, &'static str, Vec<String>)> = Vec::new();
    let mut unknown_groups: Vec<(String, Vec<String>)> = Vec::new();
    let mut canonical_items = Vec::new();
    let mut all_alias_changes: Vec<TrackChange> = Vec::new();

    for gc in &stats.genres {
        if gc.name == "(none)" || gc.name.is_empty() {
            continue;
        }
        if gc.count < min_count {
            continue;
        }

        if let Some(canonical) = genre::canonical_genre_from_alias(&gc.name) {
            let tracks = db::get_tracks_by_exact_genre(&conn, &gc.name, true).map_err(db_error)?;
            let track_ids: Vec<String> = tracks.iter().map(|t| t.id.clone()).collect();
            if stage_aliases {
                for id in &track_ids {
                    all_alias_changes.push(TrackChange {
                        track_id: id.clone(),
                        genre: Some(canonical.to_string()),
                        ..Default::default()
                    });
                }
            }
            alias_groups.push((gc.name.clone(), canonical, track_ids));
        } else if genre::is_known_genre(&gc.name) {
            canonical_items.push(serde_json::json!({
                "genre": gc.name,
                "count": gc.count,
            }));
        } else {
            let tracks = db::get_tracks_by_exact_genre(&conn, &gc.name, true).map_err(db_error)?;
            let track_ids: Vec<String> = tracks.iter().map(|t| t.id.clone()).collect();
            unknown_groups.push((gc.name.clone(), track_ids));
        }
    }

    let alias_track_count: usize = alias_groups.iter().map(|(_, _, ids)| ids.len()).sum();
    let unknown_track_count: usize = unknown_groups.iter().map(|(_, ids)| ids.len()).sum();

    // Build grouped alias output
    let alias_output: Vec<serde_json::Value> = alias_groups
        .iter()
        .map(|(current, canonical, ids)| {
            serde_json::json!({
                "from": current,
                "to": canonical,
                "count": ids.len(),
                "track_ids": ids,
            })
        })
        .collect();

    // Build grouped unknown output
    let unknown_output: Vec<serde_json::Value> = unknown_groups
        .iter()
        .map(|(current, ids)| {
            serde_json::json!({
                "genre": current,
                "count": ids.len(),
                "track_ids": ids,
            })
        })
        .collect();

    let mut result = serde_json::json!({
        "alias": alias_output,
        "unknown": unknown_output,
        "canonical": canonical_items,
        "summary": {
            "alias_tracks": alias_track_count,
            "unknown_tracks": unknown_track_count,
            "canonical_genres": canonical_items.len(),
        }
    });

    if stage_aliases && !all_alias_changes.is_empty() {
        let (staged, total_pending) = changes.stage(all_alias_changes);
        result["staging"] = serde_json::json!({
            "staged": staged,
            "total_pending": total_pending,
        });
    }

    ok_json(&result)
}

pub(super) fn handle_preview_changes(
    server: &ReklawdboxServer,
    params: PreviewChangesParams,
) -> Result<CallToolResult, McpError> {
    let mut ids = server.state.changes.pending_ids();
    if ids.is_empty() {
        return Ok(CallToolResult::success(vec![Content::text(
            "No changes staged.",
        )]));
    }

    if let Some(ref filter_ids) = params.track_ids {
        let filter_set: HashSet<&str> =
            filter_ids.iter().map(std::string::String::as_str).collect();
        ids.retain(|id| filter_set.contains(id.as_str()));
        if ids.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(
                "No staged changes for the specified track IDs.",
            )]));
        }
    }

    let conn = server.rekordbox_conn()?;
    let current_tracks = db::get_tracks_by_ids(&conn, &ids).map_err(db_error)?;

    let diffs = server.state.changes.preview(&current_tracks);
    if diffs.is_empty() {
        return Ok(CallToolResult::success(vec![Content::text(
            "Changes staged but no fields actually differ from current values.",
        )]));
    }

    let format = params.format.unwrap_or_default();
    let json = match format {
        PreviewFormat::Full => {
            serde_json::to_string(&diffs).map_err(|e| mcp_internal_error(e.to_string()))?
        }
        PreviewFormat::Summary => {
            let summary = build_preview_summary(&diffs);
            serde_json::to_string(&summary).map_err(|e| mcp_internal_error(e.to_string()))?
        }
    };
    Ok(CallToolResult::success(vec![Content::text(json)]))
}

fn build_preview_summary(diffs: &[crate::types::TrackDiff]) -> serde_json::Value {
    let total_tracks = diffs.len();
    let mut by_field: HashMap<&str, usize> = HashMap::new();
    let mut by_genre: HashMap<&str, usize> = HashMap::new();

    for diff in diffs {
        for change in &diff.changes {
            *by_field.entry(&change.field).or_default() += 1;
            if change.field == "genre" {
                *by_genre.entry(&change.new_value).or_default() += 1;
            }
        }
    }

    let total_field_changes: usize = by_field.values().sum();

    let mut by_field_sorted: Vec<_> = by_field.into_iter().collect();
    by_field_sorted.sort_by(|a, b| b.1.cmp(&a.1));

    let mut by_genre_sorted: Vec<_> = by_genre.into_iter().collect();
    by_genre_sorted.sort_by(|a, b| b.1.cmp(&a.1));

    let by_field_arr: Vec<_> = by_field_sorted
        .into_iter()
        .map(|(k, v)| serde_json::json!({"field": k, "count": v}))
        .collect();

    let mut result = serde_json::json!({
        "total_tracks": total_tracks,
        "total_field_changes": total_field_changes,
        "by_field": by_field_arr,
    });

    if !by_genre_sorted.is_empty() {
        let by_genre_arr: Vec<_> = by_genre_sorted
            .into_iter()
            .map(|(k, v)| serde_json::json!({"genre": k, "count": v}))
            .collect();
        result["by_genre"] = serde_json::json!(by_genre_arr);
    }

    result
}

pub(super) async fn handle_write_xml(
    server: &ReklawdboxServer,
    params: WriteXmlParams,
) -> Result<CallToolResult, McpError> {
    // Block export until label research from backfill_labels is acknowledged.
    let gate_count = server
        .state
        .label_research_gate
        .load(std::sync::atomic::Ordering::Relaxed);
    if gate_count > 0 && !params.skip_label_gate.unwrap_or(false) {
        return Err(mcp_internal_error(format!(
            "Label research gate: backfill_labels found {gate_count} unlabeled tracks that need research. \
             Complete step 3 of the metadata backfill SOP (research remaining label gaps) before \
             exporting. Use search_tracks(has_label=false) to find them, then research labels \
             via web search, lookup_beatport, lookup_discogs, and lookup_bandcamp.\n\n\
             Once label research is complete and remaining gaps are genuinely unresolvable, \
             call write_xml(skip_label_gate=true) to proceed."
        )));
    }

    let playlists = params.playlists.unwrap_or_default();
    let has_playlists = !playlists.is_empty();
    let snapshot = server.state.changes.take(None);
    if snapshot.is_empty() && !has_playlists {
        let result = serde_json::json!({
            "message": "No changes to write.",
            "track_count": 0,
            "changes_applied": 0,
        });
        return ok_json(&result);
    }

    let backup_status = match crate::backup::run_pre_op_backup().await {
        Ok(status) => status,
        Err(err) => {
            server.state.changes.restore(snapshot);
            return Err(mcp_internal_error(format!("pre-op {err}")));
        }
    };

    let mut ids = Vec::new();
    let mut seen_ids = HashSet::new();
    for change in &snapshot {
        if seen_ids.insert(change.track_id.clone()) {
            ids.push(change.track_id.clone());
        }
    }
    for playlist in &playlists {
        for track_id in &playlist.track_ids {
            if seen_ids.insert(track_id.clone()) {
                ids.push(track_id.clone());
            }
        }
    }

    let conn = match server.rekordbox_conn() {
        Ok(conn) => conn,
        Err(e) => {
            server.state.changes.restore(snapshot);
            return Err(e);
        }
    };
    let current_tracks = match db::get_tracks_by_ids(&conn, &ids) {
        Ok(tracks) => tracks,
        Err(e) => {
            server.state.changes.restore(snapshot);
            return Err(db_error(e));
        }
    };
    let found_ids: HashSet<&str> = current_tracks.iter().map(|t| t.id.as_str()).collect();
    let missing_ids: Vec<String> = ids
        .iter()
        .filter(|id| !found_ids.contains(id.as_str()))
        .cloned()
        .collect();
    if !missing_ids.is_empty() {
        server.state.changes.restore(snapshot);
        return Err(mcp_internal_error(format!(
            "Track IDs not found in database: {}",
            missing_ids.join(", ")
        )));
    }
    let ordered_tracks = current_tracks;
    let modified_tracks = server
        .state
        .changes
        .apply_snapshot(&ordered_tracks, &snapshot);
    let playlist_defs: Vec<xml::PlaylistDef> = playlists
        .iter()
        .map(|playlist| xml::PlaylistDef {
            name: playlist.name.clone(),
            track_ids: playlist.track_ids.clone(),
        })
        .collect();

    let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let output_path = params.output_path.map_or_else(
        || {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("reklawdbox-exports")
                .join(format!("reklawdbox-{timestamp}.xml"))
        },
        PathBuf::from,
    );

    if let Err(e) = xml::write_xml_with_playlists(&modified_tracks, &playlist_defs, &output_path) {
        server.state.changes.restore(snapshot);
        return Err(mcp_internal_error(format!("Write error: {e}")));
    }

    let track_count = modified_tracks.len();
    let changes_applied = snapshot.len();

    let mut result = serde_json::json!({
        "path": output_path.to_string_lossy(),
        "track_count": track_count,
        "changes_applied": changes_applied,
        "backup": backup_status.as_str(),
    });
    if has_playlists {
        result["playlist_count"] = serde_json::json!(playlists.len());
    }
    ok_json(&result)
}

pub(super) fn handle_clear_changes(
    changes: &ChangeManager,
    params: ClearChangesParams,
) -> Result<CallToolResult, McpError> {
    if let Some(ref fields) = params.fields {
        for f in fields {
            if crate::types::EditableField::from_str(f.as_str()).is_none() {
                return Err(McpError::invalid_params(
                    format!(
                        "unknown field '{}'. Valid fields: {}",
                        f,
                        crate::types::EditableField::all_names_csv()
                    ),
                    None,
                ));
            }
        }
        let (affected, remaining) = changes.clear_fields(params.track_ids, fields);
        let result = serde_json::json!({
            "affected": affected,
            "remaining": remaining,
            "fields_cleared": fields,
        });
        ok_json(&result)
    } else {
        let (cleared, remaining) = changes.clear(params.track_ids);
        let result = serde_json::json!({
            "cleared": cleared,
            "remaining": remaining,
        });
        ok_json(&result)
    }
}

pub(super) fn handle_clear_caches(server: &ReklawdboxServer) -> Result<CallToolResult, McpError> {
    let conn = server.cache_store_conn()?;
    let result = crate::store::clear_caches(&conn).map_err(cache_error)?;

    let staged = server.state.changes.clear(None).0;

    let json = serde_json::json!({
        "cleared": {
            "enrichment_cache": result.enrichment,
            "audio_analysis_cache": result.audio_analysis,
            "audit_issues": result.audit_issues,
            "audit_files": result.audit_files,
            "staged_changes": staged,
        },
        "preserved": ["broker_discogs_session"],
    });
    ok_json(&json)
}
