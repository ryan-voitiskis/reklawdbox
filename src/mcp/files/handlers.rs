use std::path::PathBuf;

use rmcp::ErrorData as McpError;
use rmcp::model::CallToolResult;

use crate::adapters::audio::tags;
use crate::application::files::tags as file_workflows;
use crate::mcp::{
    EmbedCoverArtParams, ExtractCoverArtParams, ReadFileTagsParams, ReklawdboxServer,
    WriteFileTagsParams, mcp_internal_error, ok_json,
};

fn workflow_error(error: file_workflows::FileWorkflowError<McpError>) -> McpError {
    match error {
        file_workflows::FileWorkflowError::Lock(error) => error,
        file_workflows::FileWorkflowError::Internal(error) => mcp_internal_error(error),
    }
}

pub(in crate::mcp) async fn handle_read_file_tags(
    server: &ReklawdboxServer,
    params: ReadFileTagsParams,
) -> Result<CallToolResult, McpError> {
    let limit = params.limit.unwrap_or(200).min(2000);
    let include_cover_art = params.include_cover_art.unwrap_or(false);
    let fields = params.fields;
    let selection = file_workflows::resolve_read_selection(
        file_workflows::ReadSelectionRequest {
            paths: params.paths,
            track_ids: params.track_ids,
            directory: params.directory,
            recursive: params.recursive.unwrap_or(false),
            glob: params.glob,
            limit,
        },
        || server.rekordbox_conn(),
    )
    .map_err(|error| match error {
        file_workflows::ReadSelectionError::InvalidSelector => McpError::invalid_params(
            "Provide exactly one of: paths, track_ids, directory".to_string(),
            None,
        ),
        file_workflows::ReadSelectionError::Connection(error) => error,
        file_workflows::ReadSelectionError::Workflow(error) => mcp_internal_error(error),
    })?;

    let output = file_workflows::read_file_tags_workflow(
        selection.file_paths,
        fields,
        include_cover_art,
        selection.inline_errors,
    )
    .await
    .map_err(mcp_internal_error)?;
    ok_json(&serde_json::json!({
        "summary": {
            "files_read": output.files_read,
            "files_failed": output.files_failed,
            "formats": output.format_counts,
        },
        "results": output.results,
    }))
}

pub(in crate::mcp) async fn handle_write_file_tags(
    server: &ReklawdboxServer,
    params: WriteFileTagsParams,
) -> Result<CallToolResult, McpError> {
    let dry_run = params.dry_run.unwrap_or(false);
    let entries = params
        .writes
        .into_iter()
        .map(|entry| tags::WriteEntry {
            path: PathBuf::from(&entry.path),
            tags: entry.tags,
            wav_targets: entry
                .wav_targets
                .unwrap_or_else(|| vec![tags::WavTarget::Id3v2, tags::WavTarget::RiffInfo]),
            comment_mode: entry.comment_mode.unwrap_or_default(),
        })
        .collect();
    let lock_server = server.clone();
    let output = file_workflows::write_file_tags_workflow(entries, dry_run, move |path| {
        lock_server.audio_file_mutation_lock(path)
    })
    .await
    .map_err(workflow_error)?;

    match output {
        file_workflows::WriteWorkflowOutput::DryRun {
            previewed,
            failed,
            results,
        } => ok_json(&serde_json::json!({
            "dry_run": true,
            "summary": {
                "files_previewed": previewed,
                "files_failed": failed,
            },
            "results": results,
        })),
        file_workflows::WriteWorkflowOutput::Written {
            files_written,
            files_failed,
            fields_written,
            results,
        } => ok_json(&serde_json::json!({
            "summary": {
                "files_written": files_written,
                "files_failed": files_failed,
                "fields_written": fields_written,
            },
            "results": results,
        })),
    }
}

pub(in crate::mcp) async fn handle_extract_cover_art(
    params: ExtractCoverArtParams,
) -> Result<CallToolResult, McpError> {
    let picture_type = params
        .picture_type
        .unwrap_or_else(|| "front_cover".to_string());
    tags::parse_picture_type(&picture_type)
        .map_err(|error| McpError::invalid_params(error.to_string(), None))?;
    let result = file_workflows::extract_cover_art_workflow(
        PathBuf::from(&params.path),
        params.output_path.map(PathBuf::from),
        picture_type,
    )
    .await
    .map_err(mcp_internal_error)?;
    ok_json(&result)
}

pub(in crate::mcp) async fn handle_embed_cover_art(
    server: &ReklawdboxServer,
    params: EmbedCoverArtParams,
) -> Result<CallToolResult, McpError> {
    let picture_type = params
        .picture_type
        .unwrap_or_else(|| "front_cover".to_string());
    tags::parse_picture_type(&picture_type)
        .map_err(|error| McpError::invalid_params(error.to_string(), None))?;
    let lock_server = server.clone();
    let output = file_workflows::embed_cover_art_workflow(
        PathBuf::from(&params.image_path),
        params.target_audio_files,
        picture_type,
        move |path| lock_server.audio_file_mutation_lock(path),
    )
    .await
    .map_err(workflow_error)?;
    ok_json(&serde_json::json!({
        "summary": {
            "files_embedded": output.files_embedded,
            "files_failed": output.files_failed,
            "image_format": output.image_format,
            "image_size_bytes": output.image_size_bytes,
        },
        "results": output.results,
    }))
}
