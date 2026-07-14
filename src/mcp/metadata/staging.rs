use std::path::PathBuf;
use std::sync::MutexGuard;

use rmcp::ErrorData as McpError;
use rmcp::model::{CallToolResult, Content};
use rusqlite::Connection;

use crate::adapters::rekordbox::xml;
use crate::application::metadata::export as metadata_export;
use crate::domain::metadata::ChangeManager;
use crate::domain::metadata::TrackChange;
use crate::mcp::{
    ClearChangesParams, PreviewChangesParams, PreviewFormat, ReklawdboxServer,
    SuggestNormalizationsParams, UpdateTracksParams, WriteXmlParams, cache_error, db_error,
    mcp_internal_error, ok_json,
};

pub(in crate::mcp) fn handle_update_tracks(
    changes: &ChangeManager,
    params: UpdateTracksParams,
) -> Result<CallToolResult, McpError> {
    let updates = params
        .changes
        .into_iter()
        .map(|change| TrackChange {
            track_id: change.track_id,
            genre: change.genre,
            comments: change.comments,
            rating: change.rating,
            color: change.color,
            label: change.label,
            year: change.year,
            album: change.album,
        })
        .collect();
    let outcome = crate::application::metadata::backfill::stage_track_updates(changes, updates)
        .map_err(|message| McpError::invalid_params(message, None))?;
    let mut result = serde_json::json!({
        "staged": outcome.staged,
        "total_pending": outcome.total_pending,
    });
    if !outcome.warnings.is_empty() {
        result["warnings"] = serde_json::json!(outcome.warnings);
    }
    ok_json(&result)
}

pub(in crate::mcp) fn handle_suggest_normalizations(
    conn: MutexGuard<'_, Connection>,
    changes: &ChangeManager,
    params: SuggestNormalizationsParams,
) -> Result<CallToolResult, McpError> {
    let result = crate::application::metadata::backfill::suggest_normalizations(
        &conn,
        changes,
        params.min_genre_count.unwrap_or(1),
        params.stage_aliases.unwrap_or(false),
    )
    .map_err(db_error)?;
    ok_json(&result)
}

pub(in crate::mcp) fn handle_preview_changes(
    server: &ReklawdboxServer,
    params: PreviewChangesParams,
) -> Result<CallToolResult, McpError> {
    let conn = server.rekordbox_conn()?;
    let outcome = metadata_export::preview_workflow(
        &conn,
        &server.context.mutation.changes,
        params.track_ids.as_deref(),
    )
    .map_err(db_error)?;
    let diffs = match outcome {
        metadata_export::PreviewWorkflowOutcome::NoChangesStaged => {
            return Ok(CallToolResult::success(vec![Content::text(
                "No changes staged.",
            )]));
        }
        metadata_export::PreviewWorkflowOutcome::NoMatchingChanges => {
            return Ok(CallToolResult::success(vec![Content::text(
                "No staged changes for the specified track IDs.",
            )]));
        }
        metadata_export::PreviewWorkflowOutcome::NoDifferences => {
            return Ok(CallToolResult::success(vec![Content::text(
                "Changes staged but no fields actually differ from current values.",
            )]));
        }
        metadata_export::PreviewWorkflowOutcome::Differences(diffs) => diffs,
    };
    let json = match params.format.unwrap_or_default() {
        PreviewFormat::Full => {
            serde_json::to_string(&diffs).map_err(|error| mcp_internal_error(error.to_string()))?
        }
        PreviewFormat::Summary => {
            serde_json::to_string(&metadata_export::build_preview_summary(&diffs))
                .map_err(|error| mcp_internal_error(error.to_string()))?
        }
    };
    Ok(CallToolResult::success(vec![Content::text(json)]))
}

pub(in crate::mcp) async fn handle_write_xml(
    server: &ReklawdboxServer,
    params: WriteXmlParams,
) -> Result<CallToolResult, McpError> {
    let playlists = params
        .playlists
        .unwrap_or_default()
        .into_iter()
        .map(|playlist| xml::PlaylistDef {
            name: playlist.name,
            track_ids: playlist.track_ids,
        })
        .collect();
    let outcome = metadata_export::export_changes(
        &server.context.mutation.changes,
        &server.context.mutation.label_research_gate,
        &server.context.mutation.xml_export_lock,
        params.skip_label_gate.unwrap_or(false),
        || {
            server
                .effective_db_path()
                .map_err(|error| error.message.to_string())
        },
        |ids| {
            let conn = server
                .rekordbox_conn()
                .map_err(|error| error.message.to_string())?;
            crate::adapters::rekordbox::get_tracks_by_ids(&conn, ids)
                .map_err(|error| format!("DB error: {error}"))
        },
        params.output_path.map(PathBuf::from),
        playlists,
    )
    .await
    .map_err(|error| match error {
        metadata_export::ExportError::LabelGate(gate_count) => mcp_internal_error(format!(
            "Label research gate: backfill_labels found {gate_count} unlabeled tracks that need research. \
             Complete Step 1c of the metadata backfill SOP (research remaining label gaps) before \
             exporting. Use search_tracks(has_label=false) to find them, then research labels \
             via web search, lookup_discogs, lookup_bandcamp, and lookup_musicbrainz.\n\n\
             Once label research is complete and remaining gaps are genuinely unresolvable, \
             call write_xml(skip_label_gate=true) to proceed."
        )),
        metadata_export::ExportError::Workflow(message) => mcp_internal_error(message),
    })?;

    match outcome {
        metadata_export::ExportOutcome::NoChanges => ok_json(&serde_json::json!({
            "message": "No changes to write.",
            "track_count": 0,
            "changes_applied": 0,
        })),
        metadata_export::ExportOutcome::Written {
            path,
            track_count,
            changes_applied,
            backup,
            playlist_count,
        } => {
            let mut result = serde_json::json!({
                "path": path.to_string_lossy(),
                "track_count": track_count,
                "changes_applied": changes_applied,
                "backup": backup,
            });
            if playlist_count > 0 {
                result["playlist_count"] = serde_json::json!(playlist_count);
            }
            ok_json(&result)
        }
    }
}

pub(in crate::mcp) fn handle_clear_changes(
    changes: &ChangeManager,
    params: ClearChangesParams,
) -> Result<CallToolResult, McpError> {
    let fields = params.fields;
    let result = metadata_export::clear_changes(changes, params.track_ids, fields.as_deref())
        .map_err(|message| McpError::invalid_params(message, None))?;
    ok_json(&result)
}

pub(in crate::mcp) fn handle_clear_caches(
    server: &ReklawdboxServer,
) -> Result<CallToolResult, McpError> {
    let conn = server.cache_store_conn()?;
    let result = crate::adapters::state::clear_caches(&conn).map_err(cache_error)?;

    let staged = server.context.mutation.changes.clear(None).0;

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
