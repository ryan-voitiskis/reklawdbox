use std::collections::HashMap;
use std::path::PathBuf;

use rmcp::ErrorData as McpError;
use rmcp::model::{CallToolResult, Content};

use super::*;
use crate::db;
use crate::tags;

pub(super) async fn handle_read_file_tags(
    server: &ReklawdboxServer,
    params: ReadFileTagsParams,
) -> Result<CallToolResult, McpError> {
    // Validate exactly one selector
    let selector_count = [
        params.paths.is_some(),
        params.track_ids.is_some(),
        params.directory.is_some(),
    ]
    .iter()
    .filter(|&&v| v)
    .count();
    if selector_count != 1 {
        return Err(McpError::invalid_params(
            "Provide exactly one of: paths, track_ids, directory".to_string(),
            None,
        ));
    }

    let limit = params.limit.unwrap_or(200).min(2000);
    let include_cover_art = params.include_cover_art.unwrap_or(false);
    let fields = params.fields;

    // Resolve file paths from the chosen selector
    let mut inline_errors: Vec<tags::FileReadResult> = Vec::new();
    let mut file_paths: Vec<String> = if let Some(paths) = params.paths {
        paths
    } else if let Some(track_ids) = params.track_ids {
        let conn = server.rekordbox_conn()?;
        let mut resolved = Vec::with_capacity(track_ids.len());
        for id in &track_ids {
            match db::get_track(&conn, id) {
                Ok(Some(track)) => match resolve_file_path(&track.file_path) {
                    Ok(path) => resolved.push(path),
                    Err(e) => inline_errors.push(tags::FileReadResult::Error {
                        path: format!("track_id:{id}"),
                        error: format!("Failed to resolve path: {e}"),
                    }),
                },
                Ok(None) => {
                    inline_errors.push(tags::FileReadResult::Error {
                        path: format!("track_id:{id}"),
                        error: format!("Track '{id}' not found"),
                    });
                }
                Err(e) => {
                    inline_errors.push(tags::FileReadResult::Error {
                        path: format!("track_id:{id}"),
                        error: format!("DB error: {e}"),
                    });
                }
            }
        }
        resolved
    } else if let Some(directory) = params.directory {
        let recursive = params.recursive.unwrap_or(false);
        let glob_pattern = params.glob.clone();
        scan_audio_directory(&directory, recursive, glob_pattern.as_deref())
            .map_err(mcp_internal_error)?
    } else {
        unreachable!()
    };

    file_paths.truncate(limit);

    // Read tags concurrently with a semaphore (max 8 concurrent)
    let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(8));
    let mut handles = Vec::with_capacity(file_paths.len());

    for file_path in file_paths {
        let sem = semaphore.clone();
        let fields_clone = fields.clone();
        handles.push(tokio::task::spawn(async move {
            let _permit = sem.acquire().await.unwrap();
            let path = std::path::PathBuf::from(&file_path);
            let selected_fields = fields_clone;
            tokio::task::spawn_blocking(move || {
                tags::read_file_tags(&path, selected_fields.as_deref(), include_cover_art)
            })
            .await
            .unwrap_or_else(|e| tags::FileReadResult::Error {
                path: file_path,
                error: format!("task join error: {e}"),
            })
        }));
    }

    let mut results = Vec::with_capacity(inline_errors.len() + handles.len());
    let mut files_read: usize = 0;
    let mut files_failed: usize = inline_errors.len();
    let mut format_counts: HashMap<String, usize> = HashMap::new();

    // Prepend any inline errors from track_id resolution
    results.append(&mut inline_errors);

    for handle in handles {
        let result = handle
            .await
            .map_err(|e| mcp_internal_error(format!("join error: {e}")))?;
        match &result {
            tags::FileReadResult::Single { format, .. } => {
                files_read += 1;
                *format_counts.entry(format.clone()).or_insert(0) += 1;
            }
            tags::FileReadResult::Wav { format, .. } => {
                files_read += 1;
                *format_counts.entry(format.clone()).or_insert(0) += 1;
            }
            tags::FileReadResult::Error { .. } => {
                files_failed += 1;
            }
        }
        results.push(result);
    }

    let output = serde_json::json!({
        "summary": {
            "files_read": files_read,
            "files_failed": files_failed,
            "formats": format_counts,
        },
        "results": results,
    });

    let json =
        serde_json::to_string_pretty(&output).map_err(|e| mcp_internal_error(format!("{e}")))?;
    Ok(CallToolResult::success(vec![Content::text(json)]))
}

pub(super) async fn handle_write_file_tags(
    params: WriteFileTagsParams,
) -> Result<CallToolResult, McpError> {
    let dry_run = params.dry_run.unwrap_or(false);

    // Build WriteEntry values (per-entry validation happens in tags::write_file_tags)
    let entries: Vec<tags::WriteEntry> = params
        .writes
        .into_iter()
        .map(|e| tags::WriteEntry {
            path: PathBuf::from(&e.path),
            tags: e.tags,
            wav_targets: e
                .wav_targets
                .unwrap_or_else(|| vec![tags::WavTarget::Id3v2, tags::WavTarget::RiffInfo]),
        })
        .collect();

    if dry_run {
        // Dry-run is read-only; entries are currently processed sequentially.
        let mut results = Vec::with_capacity(entries.len());
        for entry in entries {
            let result = tokio::task::spawn_blocking(move || tags::write_file_tags_dry_run(&entry))
                .await
                .map_err(|e| mcp_internal_error(format!("join error: {e}")))?;
            results.push(result);
        }

        let mut previewed: usize = 0;
        let mut failed: usize = 0;
        for r in &results {
            match r {
                tags::FileDryRunResult::Preview { .. } => previewed += 1,
                tags::FileDryRunResult::Error { .. } => failed += 1,
            }
        }

        let output = serde_json::json!({
            "dry_run": true,
            "summary": {
                "files_previewed": previewed,
                "files_failed": failed,
            },
            "results": results,
        });

        let json = serde_json::to_string_pretty(&output)
            .map_err(|e| mcp_internal_error(format!("{e}")))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    } else {
        // Actual writes: sequential
        let mut results = Vec::with_capacity(entries.len());
        let mut files_written: usize = 0;
        let mut files_failed: usize = 0;
        let mut total_fields_written: usize = 0;

        for entry in entries {
            let result = tokio::task::spawn_blocking(move || tags::write_file_tags(&entry))
                .await
                .map_err(|e| mcp_internal_error(format!("join error: {e}")))?;

            match &result {
                tags::FileWriteResult::Ok { fields_written, .. } => {
                    files_written += 1;
                    total_fields_written += fields_written.len();
                }
                tags::FileWriteResult::Error { .. } => files_failed += 1,
            }
            results.push(result);
        }

        let output = serde_json::json!({
            "summary": {
                "files_written": files_written,
                "files_failed": files_failed,
                "fields_written": total_fields_written,
            },
            "results": results,
        });

        let json = serde_json::to_string_pretty(&output)
            .map_err(|e| mcp_internal_error(format!("{e}")))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }
}

pub(super) async fn handle_extract_cover_art(
    params: ExtractCoverArtParams,
) -> Result<CallToolResult, McpError> {
    let path = PathBuf::from(&params.path);
    let output_path = params.output_path.map(PathBuf::from);
    let picture_type = params
        .picture_type
        .unwrap_or_else(|| "front_cover".to_string());

    let result = tokio::task::spawn_blocking(move || {
        tags::extract_cover_art(&path, output_path.as_deref(), &picture_type)
    })
    .await
    .map_err(|e| mcp_internal_error(format!("join error: {e}")))?
    .map_err(|e| mcp_internal_error(e.to_string()))?;

    let json =
        serde_json::to_string_pretty(&result).map_err(|e| mcp_internal_error(format!("{e}")))?;
    Ok(CallToolResult::success(vec![Content::text(json)]))
}

pub(super) async fn handle_embed_cover_art(
    params: EmbedCoverArtParams,
) -> Result<CallToolResult, McpError> {
    let image_path = PathBuf::from(&params.image_path);
    let picture_type = params
        .picture_type
        .unwrap_or_else(|| "front_cover".to_string());

    // Read image metadata before the embed loop
    let image_size_bytes = std::fs::metadata(&image_path)
        .map(|m| m.len() as usize)
        .unwrap_or(0);
    let image_format = {
        let data = std::fs::read(&image_path)
            .map_err(|e| mcp_internal_error(format!("Failed to read image: {e}")))?;
        if data.starts_with(&[0xff, 0xd8]) {
            "jpeg"
        } else if data.starts_with(&[0x89, 0x50, 0x4e, 0x47]) {
            "png"
        } else if data.starts_with(&[0x47, 0x49, 0x46]) {
            "gif"
        } else if data.starts_with(&[0x42, 0x4d]) {
            "bmp"
        } else {
            "unknown"
        }
    };

    let mut results = Vec::with_capacity(params.target_audio_files.len());
    let mut files_embedded: usize = 0;
    let mut files_failed: usize = 0;

    // Sequential writes to avoid file contention
    for target in params.target_audio_files {
        let img = image_path.clone();
        let tgt = PathBuf::from(&target);
        let pt = picture_type.clone();
        let result = tokio::task::spawn_blocking(move || tags::embed_cover_art(&img, &tgt, &pt))
            .await
            .map_err(|e| mcp_internal_error(format!("join error: {e}")))?;

        match &result {
            tags::FileEmbedResult::Ok { .. } => files_embedded += 1,
            tags::FileEmbedResult::Error { .. } => files_failed += 1,
        }
        results.push(result);
    }

    let output = serde_json::json!({
        "summary": {
            "files_embedded": files_embedded,
            "files_failed": files_failed,
            "image_format": image_format,
            "image_size_bytes": image_size_bytes,
        },
        "results": results,
    });

    let json =
        serde_json::to_string_pretty(&output).map_err(|e| mcp_internal_error(format!("{e}")))?;
    Ok(CallToolResult::success(vec![Content::text(json)]))
}
