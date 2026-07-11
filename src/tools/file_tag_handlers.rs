use std::collections::HashMap;
use std::path::PathBuf;

use rmcp::ErrorData as McpError;
use rmcp::model::CallToolResult;

use super::*;
use crate::db;
use crate::tags;

pub(super) async fn handle_read_file_tags(
    server: &ReklawdboxServer,
    params: ReadFileTagsParams,
) -> Result<CallToolResult, McpError> {
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

    let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(8));
    let mut handles = Vec::with_capacity(file_paths.len());

    for file_path in file_paths {
        let sem = semaphore.clone();
        let fields_clone = fields.clone();
        handles.push(tokio::task::spawn(async move {
            let _permit = sem.acquire().await.expect("semaphore is never closed");
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

    ok_json(&output)
}

pub(super) async fn handle_write_file_tags(
    server: &ReklawdboxServer,
    params: WriteFileTagsParams,
) -> Result<CallToolResult, McpError> {
    let dry_run = params.dry_run.unwrap_or(false);

    let entries: Vec<tags::WriteEntry> = params
        .writes
        .into_iter()
        .map(|e| tags::WriteEntry {
            path: PathBuf::from(&e.path),
            tags: e.tags,
            wav_targets: e
                .wav_targets
                .unwrap_or_else(|| vec![tags::WavTarget::Id3v2, tags::WavTarget::RiffInfo]),
            comment_mode: e.comment_mode.unwrap_or_default(),
        })
        .collect();

    if dry_run {
        let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(8));
        let mut handles = Vec::with_capacity(entries.len());

        for entry in entries {
            let sem = semaphore.clone();
            let path_display = entry.path.display().to_string();
            handles.push(tokio::task::spawn(async move {
                let _permit = sem.acquire().await.expect("semaphore is never closed");
                tokio::task::spawn_blocking(move || tags::write_file_tags_dry_run(&entry))
                    .await
                    .unwrap_or_else(|e| tags::FileDryRunResult::Error {
                        path: path_display,
                        status: "error".to_string(),
                        error: format!("task join error: {e}"),
                    })
            }));
        }

        let mut results = Vec::with_capacity(handles.len());
        let mut previewed: usize = 0;
        let mut failed: usize = 0;

        for handle in handles {
            let result = handle
                .await
                .map_err(|e| mcp_internal_error(format!("join error: {e}")))?;
            match &result {
                tags::FileDryRunResult::Preview { .. } => previewed += 1,
                tags::FileDryRunResult::Error { .. } => failed += 1,
            }
            results.push(result);
        }

        let output = serde_json::json!({
            "dry_run": true,
            "summary": {
                "files_previewed": previewed,
                "files_failed": failed,
            },
            "results": results,
        });

        ok_json(&output)
    } else {
        let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(8));
        let entry_count = entries.len();
        let mut results: Vec<Option<tags::FileWriteResult>> =
            (0..entry_count).map(|_| None).collect();
        let mut groups: Vec<(PathBuf, Vec<(usize, tags::WriteEntry)>)> = Vec::new();
        let mut group_indices: HashMap<PathBuf, usize> = HashMap::new();

        for (index, entry) in entries.into_iter().enumerate() {
            let path_display = entry.path.display().to_string();
            match tokio::fs::canonicalize(&entry.path).await {
                Ok(canonical_path) => {
                    if let Some(&group_index) = group_indices.get(&canonical_path) {
                        groups[group_index].1.push((index, entry));
                    } else {
                        let group_index = groups.len();
                        group_indices.insert(canonical_path.clone(), group_index);
                        groups.push((canonical_path, vec![(index, entry)]));
                    }
                }
                Err(error) => {
                    results[index] = Some(tags::FileWriteResult::Error {
                        path: path_display,
                        status: "error".to_string(),
                        error: format!("Failed to canonicalize path: {error}"),
                    });
                }
            }
        }

        let mut handles = Vec::with_capacity(groups.len());
        for (canonical_path, entries) in groups {
            let sem = semaphore.clone();
            let server = server.clone();
            handles.push(tokio::task::spawn(async move {
                let _permit = sem.acquire().await.expect("semaphore is never closed");
                let mutation_lock = server.audio_file_mutation_lock(&canonical_path)?;
                let _guard = mutation_lock.lock().await;
                let mut group_results = Vec::with_capacity(entries.len());

                for (index, entry) in entries {
                    let path_display = entry.path.display().to_string();
                    let result = tokio::task::spawn_blocking(move || tags::write_file_tags(&entry))
                        .await
                        .unwrap_or_else(|e| tags::FileWriteResult::Error {
                            path: path_display,
                            status: "error".to_string(),
                            error: format!("task join error: {e}"),
                        });
                    group_results.push((index, result));
                }

                Ok::<_, McpError>(group_results)
            }));
        }

        for handle in handles {
            let group_results = handle
                .await
                .map_err(|e| mcp_internal_error(format!("join error: {e}")))??;
            for (index, result) in group_results {
                results[index] = Some(result);
            }
        }

        let results: Vec<tags::FileWriteResult> =
            results
                .into_iter()
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| mcp_internal_error("missing file tag write result"))?;
        let mut files_written: usize = 0;
        let mut files_failed: usize = 0;
        let mut total_fields_written: usize = 0;

        for result in &results {
            match &result {
                tags::FileWriteResult::Ok { fields_written, .. } => {
                    files_written += 1;
                    total_fields_written += fields_written.len();
                }
                tags::FileWriteResult::Error { .. } => files_failed += 1,
            }
        }

        let output = serde_json::json!({
            "summary": {
                "files_written": files_written,
                "files_failed": files_failed,
                "fields_written": total_fields_written,
            },
            "results": results,
        });

        ok_json(&output)
    }
}

pub(super) async fn handle_extract_cover_art(
    params: ExtractCoverArtParams,
) -> Result<CallToolResult, McpError> {
    let picture_type = params
        .picture_type
        .unwrap_or_else(|| "front_cover".to_string());
    tags::parse_picture_type(&picture_type)
        .map_err(|error| McpError::invalid_params(error.to_string(), None))?;
    let path = PathBuf::from(&params.path);
    let output_path = params.output_path.map(PathBuf::from);

    let result = tokio::task::spawn_blocking(move || {
        tags::extract_cover_art(&path, output_path.as_deref(), &picture_type)
    })
    .await
    .map_err(|e| mcp_internal_error(format!("join error: {e}")))?
    .map_err(|e| mcp_internal_error(e.to_string()))?;

    ok_json(&result)
}

pub(super) async fn handle_embed_cover_art(
    server: &ReklawdboxServer,
    params: EmbedCoverArtParams,
) -> Result<CallToolResult, McpError> {
    let picture_type = params
        .picture_type
        .unwrap_or_else(|| "front_cover".to_string());
    tags::parse_picture_type(&picture_type)
        .map_err(|error| McpError::invalid_params(error.to_string(), None))?;
    let image_path = PathBuf::from(&params.image_path);

    let image_size_bytes = tokio::fs::metadata(&image_path)
        .await
        .map(|m| m.len() as usize)
        .unwrap_or(0);
    let image_format = {
        let data = tokio::fs::read(&image_path)
            .await
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

    let targets = params.target_audio_files;
    let target_count = targets.len();
    let mut results: Vec<Option<tags::FileEmbedResult>> = (0..target_count).map(|_| None).collect();
    let mut groups: Vec<(PathBuf, Vec<(usize, PathBuf)>)> = Vec::new();
    let mut group_indices: HashMap<PathBuf, usize> = HashMap::new();

    for (index, target) in targets.into_iter().enumerate() {
        let target_path = PathBuf::from(&target);
        match tokio::fs::canonicalize(&target_path).await {
            Ok(canonical_path) => {
                if let Some(&group_index) = group_indices.get(&canonical_path) {
                    groups[group_index].1.push((index, target_path));
                } else {
                    let group_index = groups.len();
                    group_indices.insert(canonical_path.clone(), group_index);
                    groups.push((canonical_path, vec![(index, target_path)]));
                }
            }
            Err(error) => {
                results[index] = Some(tags::FileEmbedResult::Error {
                    path: target,
                    status: "error".to_string(),
                    error: format!("Failed to canonicalize path: {error}"),
                });
            }
        }
    }

    let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(8));
    let mut handles = Vec::with_capacity(groups.len());

    for (canonical_path, targets) in groups {
        let sem = semaphore.clone();
        let img = image_path.clone();
        let pt = picture_type.clone();
        let server = server.clone();
        handles.push(tokio::task::spawn(async move {
            let _permit = sem.acquire().await.expect("semaphore is never closed");
            let mutation_lock = server.audio_file_mutation_lock(&canonical_path)?;
            let _guard = mutation_lock.lock().await;
            let mut group_results = Vec::with_capacity(targets.len());

            for (index, target) in targets {
                let target_display = target.display().to_string();
                let image_path = img.clone();
                let picture_type = pt.clone();
                let result = tokio::task::spawn_blocking(move || {
                    tags::embed_cover_art(&image_path, &target, &picture_type)
                })
                .await
                .unwrap_or_else(|e| tags::FileEmbedResult::Error {
                    path: target_display,
                    status: "error".to_string(),
                    error: format!("task join error: {e}"),
                });
                group_results.push((index, result));
            }

            Ok::<_, McpError>(group_results)
        }));
    }

    for handle in handles {
        let group_results = handle
            .await
            .map_err(|e| mcp_internal_error(format!("join error: {e}")))??;
        for (index, result) in group_results {
            results[index] = Some(result);
        }
    }

    let results: Vec<tags::FileEmbedResult> = results
        .into_iter()
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| mcp_internal_error("missing cover art embed result"))?;
    let mut files_embedded: usize = 0;
    let mut files_failed: usize = 0;

    for result in &results {
        match result {
            tags::FileEmbedResult::Ok { .. } => files_embedded += 1,
            tags::FileEmbedResult::Error { .. } => files_failed += 1,
        }
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

    ok_json(&output)
}
