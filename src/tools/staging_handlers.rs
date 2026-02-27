use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::MutexGuard;

use rmcp::model::{CallToolResult, Content};
use rmcp::ErrorData as McpError;
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
        if let Some(r) = c.rating
            && (r == 0 || r > 5)
        {
            return Err(McpError::invalid_params(
                format!("rating must be 1-5, got {r}"),
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
            warnings.push(format!("'{}' is not in the genre taxonomy", g));
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
        })
        .collect();

    let echo: Vec<serde_json::Value> = track_changes
        .iter()
        .map(|c| {
            serde_json::json!({
                "track_id": c.track_id,
                "genre": c.genre,
                "comments": c.comments,
                "rating": c.rating,
                "color": c.color,
            })
        })
        .collect();

    let (staged, total) = changes.stage(track_changes);
    let mut result = serde_json::json!({
        "staged": staged,
        "total_pending": total,
        "changes": echo,
    });
    if !warnings.is_empty() {
        result["warnings"] = serde_json::json!(warnings);
    }
    attach_corpus_provenance(&mut result, consult_genre_workflow_docs());
    let json =
        serde_json::to_string_pretty(&result).map_err(|e| mcp_internal_error(format!("{e}")))?;
    Ok(CallToolResult::success(vec![Content::text(json)]))
}

pub(super) fn handle_suggest_normalizations(
    conn: MutexGuard<'_, Connection>,
    params: SuggestNormalizationsParams,
) -> Result<CallToolResult, McpError> {
    let min_count = params.min_genre_count.unwrap_or(1);

    let stats =
        db::get_library_stats(&conn).map_err(|e| mcp_internal_error(format!("DB error: {e}")))?;

    let mut alias_suggestions = Vec::new();
    let mut unknown_items = Vec::new();
    let mut canonical_items = Vec::new();

    for gc in &stats.genres {
        if gc.name == "(none)" || gc.name.is_empty() {
            continue;
        }
        if gc.count < min_count {
            continue;
        }

        if let Some(canonical) = genre::canonical_genre_from_alias(&gc.name) {
            let tracks = db::get_tracks_by_exact_genre(&conn, &gc.name, true)
                .map_err(|e| mcp_internal_error(format!("DB error: {e}")))?;
            for t in tracks {
                alias_suggestions.push(crate::types::NormalizationSuggestion {
                    track_id: t.id,
                    title: t.title,
                    artist: t.artist,
                    current_genre: gc.name.clone(),
                    suggested_genre: Some(canonical.to_string()),
                    confidence: crate::types::Confidence::Alias,
                });
            }
        } else if genre::is_known_genre(&gc.name) {
            canonical_items.push(serde_json::json!({
                "genre": gc.name,
                "count": gc.count,
            }));
        } else {
            let tracks = db::get_tracks_by_exact_genre(&conn, &gc.name, true)
                .map_err(|e| mcp_internal_error(format!("DB error: {e}")))?;
            for t in tracks {
                unknown_items.push(crate::types::NormalizationSuggestion {
                    track_id: t.id,
                    title: t.title,
                    artist: t.artist,
                    current_genre: gc.name.clone(),
                    suggested_genre: None,
                    confidence: crate::types::Confidence::Unknown,
                });
            }
        }
    }

    let mut result = serde_json::json!({
        "alias": alias_suggestions,
        "unknown": unknown_items,
        "canonical": canonical_items,
        "summary": {
            "alias_tracks": alias_suggestions.len(),
            "unknown_tracks": unknown_items.len(),
            "canonical_genres": canonical_items.len(),
        }
    });
    attach_corpus_provenance(&mut result, consult_genre_workflow_docs());
    let json =
        serde_json::to_string_pretty(&result).map_err(|e| mcp_internal_error(format!("{e}")))?;
    Ok(CallToolResult::success(vec![Content::text(json)]))
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

    // Filter to requested track IDs if provided
    if let Some(ref filter_ids) = params.track_ids {
        let filter_set: HashSet<&str> = filter_ids.iter().map(|s| s.as_str()).collect();
        ids.retain(|id| filter_set.contains(id.as_str()));
        if ids.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(
                "No staged changes for the specified track IDs.",
            )]));
        }
    }

    let conn = server.rekordbox_conn()?;
    let current_tracks =
        db::get_tracks_by_ids(&conn, &ids).map_err(|e| mcp_internal_error(format!("DB error: {e}")))?;

    let diffs = server.state.changes.preview(&current_tracks);
    if diffs.is_empty() {
        return Ok(CallToolResult::success(vec![Content::text(
            "Changes staged but no fields actually differ from current values.",
        )]));
    }

    let json =
        serde_json::to_string_pretty(&diffs).map_err(|e| mcp_internal_error(format!("{e}")))?;
    Ok(CallToolResult::success(vec![Content::text(json)]))
}

pub(super) async fn handle_write_xml(
    server: &ReklawdboxServer,
    params: WriteXmlParams,
) -> Result<CallToolResult, McpError> {
    let playlists = params.playlists.unwrap_or_default();
    let has_playlists = !playlists.is_empty();
    let snapshot = server.state.changes.take(None);
    if snapshot.is_empty() && !has_playlists {
        let mut result = serde_json::json!({
            "message": "No changes to write.",
            "track_count": 0,
            "changes_applied": 0,
        });
        attach_corpus_provenance(&mut result, consult_xml_workflow_docs());
        let json =
            serde_json::to_string_pretty(&result).map_err(|e| mcp_internal_error(format!("{e}")))?;
        return Ok(CallToolResult::success(vec![Content::text(json)]));
    }

    let backup_script_env = std::env::var("REKLAWDBOX_BACKUP_SCRIPT")
        .ok()
        .filter(|v| !v.trim().is_empty());
    let backup_script_candidates: Vec<String> = backup_script_env
        .into_iter()
        .chain(["scripts/backup.sh".to_string(), "backup.sh".to_string()])
        .collect();
    let backup_script = backup_script_candidates
        .iter()
        .find(|path| std::path::Path::new(path).exists());
    if let Some(script_path) = backup_script {
        tracing::info!("Running pre-op backup...");
        let script_path = script_path.to_string();
        match tokio::task::spawn_blocking(move || {
            std::process::Command::new("bash")
                .arg(&script_path)
                .arg("--pre-op")
                .output()
        })
        .await
        {
            Ok(Ok(backup_output)) if backup_output.status.success() => {
                tracing::info!("Backup completed.")
            }
            Ok(Ok(backup_output)) => {
                let stderr_out = String::from_utf8_lossy(&backup_output.stderr);
                tracing::warn!("Backup warning: {stderr_out}");
            }
            Ok(Err(e)) => tracing::warn!("Backup skipped: {e}"),
            Err(e) => tracing::warn!("Backup task failed: {e}"),
        }
    }

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
            return Err(mcp_internal_error(format!("DB error: {e}")));
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
    let output_path = params.output_path.map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(format!("rekordbox-exports/reklawdbox-{timestamp}.xml"))
    });

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
    });
    if has_playlists {
        result["playlist_count"] = serde_json::json!(playlists.len());
    }
    attach_corpus_provenance(&mut result, consult_xml_workflow_docs());
    let json =
        serde_json::to_string_pretty(&result).map_err(|e| mcp_internal_error(format!("{e}")))?;
    Ok(CallToolResult::success(vec![Content::text(json)]))
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
        let json =
            serde_json::to_string_pretty(&result).map_err(|e| mcp_internal_error(format!("{e}")))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    } else {
        let (cleared, remaining) = changes.clear(params.track_ids);
        let result = serde_json::json!({
            "cleared": cleared,
            "remaining": remaining,
        });
        let json =
            serde_json::to_string_pretty(&result).map_err(|e| mcp_internal_error(format!("{e}")))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }
}
