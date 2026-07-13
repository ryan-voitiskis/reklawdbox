//! Direct audio-tag workflow policy and mutation serialization.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::adapters::audio::tags;
use crate::adapters::{audio, rekordbox};

const FILE_CONCURRENCY: usize = 8;

#[derive(Debug)]
pub(crate) struct ReadSelectionRequest {
    pub(crate) paths: Option<Vec<String>>,
    pub(crate) track_ids: Option<Vec<String>>,
    pub(crate) directory: Option<String>,
    pub(crate) recursive: bool,
    pub(crate) glob: Option<String>,
    pub(crate) limit: usize,
}

#[derive(Debug)]
pub(crate) enum ReadSelectionError<E> {
    InvalidSelector,
    Connection(E),
    Workflow(String),
}

#[derive(Debug)]
pub(crate) struct ReadSelection {
    pub(crate) file_paths: Vec<String>,
    pub(crate) inline_errors: Vec<tags::FileReadResult>,
}

pub(crate) fn resolve_read_selection<E, F, G>(
    request: ReadSelectionRequest,
    open_rekordbox: F,
) -> Result<ReadSelection, ReadSelectionError<E>>
where
    F: FnOnce() -> Result<G, E>,
    G: std::ops::Deref<Target = rusqlite::Connection>,
{
    let selector_count = [
        request.paths.is_some(),
        request.track_ids.is_some(),
        request.directory.is_some(),
    ]
    .into_iter()
    .filter(|selected| *selected)
    .count();
    if selector_count != 1 {
        return Err(ReadSelectionError::InvalidSelector);
    }

    let mut inline_errors = Vec::new();
    let mut file_paths = if let Some(paths) = request.paths {
        paths
    } else if let Some(track_ids) = request.track_ids {
        let conn = open_rekordbox().map_err(ReadSelectionError::Connection)?;
        let mut resolved = Vec::with_capacity(track_ids.len());
        for id in &track_ids {
            match rekordbox::get_track(&conn, id) {
                Ok(Some(track)) => match audio::resolve_audio_path(&track.file_path) {
                    Ok(path) => resolved.push(path),
                    Err(error) => inline_errors.push(tags::FileReadResult::Error {
                        path: format!("track_id:{id}"),
                        error: format!("Failed to resolve path: {error}"),
                    }),
                },
                Ok(None) => inline_errors.push(tags::FileReadResult::Error {
                    path: format!("track_id:{id}"),
                    error: format!("Track '{id}' not found"),
                }),
                Err(error) => inline_errors.push(tags::FileReadResult::Error {
                    path: format!("track_id:{id}"),
                    error: format!("DB error: {error}"),
                }),
            }
        }
        resolved
    } else if let Some(directory) = request.directory {
        audio::scan_audio_directory(&directory, request.recursive, request.glob.as_deref())
            .map_err(|error| ReadSelectionError::Workflow(error.to_string()))?
    } else {
        unreachable!()
    };
    file_paths.truncate(request.limit);
    Ok(ReadSelection {
        file_paths,
        inline_errors,
    })
}

#[derive(Debug)]
pub(crate) struct ReadWorkflowOutput {
    pub(crate) files_read: usize,
    pub(crate) files_failed: usize,
    pub(crate) format_counts: HashMap<String, usize>,
    pub(crate) results: Vec<tags::FileReadResult>,
}

#[derive(Debug)]
pub(crate) enum WriteWorkflowOutput {
    DryRun {
        previewed: usize,
        failed: usize,
        results: Vec<tags::FileDryRunResult>,
    },
    Written {
        files_written: usize,
        files_failed: usize,
        fields_written: usize,
        results: Vec<tags::FileWriteResult>,
    },
}

#[derive(Debug)]
pub(crate) struct EmbedWorkflowOutput {
    pub(crate) files_embedded: usize,
    pub(crate) files_failed: usize,
    pub(crate) image_format: &'static str,
    pub(crate) image_size_bytes: usize,
    pub(crate) results: Vec<tags::FileEmbedResult>,
}

#[derive(Debug)]
pub(crate) enum FileWorkflowError<E> {
    Lock(E),
    Internal(String),
}

pub(crate) async fn read_file_tags_workflow(
    file_paths: Vec<String>,
    fields: Option<Vec<String>>,
    include_cover_art: bool,
    mut inline_errors: Vec<tags::FileReadResult>,
) -> Result<ReadWorkflowOutput, String> {
    let semaphore = Arc::new(tokio::sync::Semaphore::new(FILE_CONCURRENCY));
    let mut handles = Vec::with_capacity(file_paths.len());
    for file_path in file_paths {
        let semaphore = semaphore.clone();
        let fields = fields.clone();
        handles.push(tokio::task::spawn(async move {
            let _permit = semaphore
                .acquire()
                .await
                .expect("semaphore is never closed");
            let path = PathBuf::from(&file_path);
            tokio::task::spawn_blocking(move || {
                tags::read_file_tags(&path, fields.as_deref(), include_cover_art)
            })
            .await
            .unwrap_or_else(|error| tags::FileReadResult::Error {
                path: file_path,
                error: format!("task join error: {error}"),
            })
        }));
    }

    let mut results = Vec::with_capacity(inline_errors.len() + handles.len());
    let mut files_read = 0;
    let mut files_failed = inline_errors.len();
    let mut format_counts = HashMap::new();
    results.append(&mut inline_errors);
    for handle in handles {
        let result = handle
            .await
            .map_err(|error| format!("join error: {error}"))?;
        match &result {
            tags::FileReadResult::Single { format, .. }
            | tags::FileReadResult::Wav { format, .. } => {
                files_read += 1;
                *format_counts.entry(format.clone()).or_insert(0) += 1;
            }
            tags::FileReadResult::Error { .. } => files_failed += 1,
        }
        results.push(result);
    }
    Ok(ReadWorkflowOutput {
        files_read,
        files_failed,
        format_counts,
        results,
    })
}

pub(crate) async fn extract_cover_art_workflow(
    path: PathBuf,
    output_path: Option<PathBuf>,
    picture_type: String,
) -> Result<tags::ExtractArtResult, String> {
    tokio::task::spawn_blocking(move || {
        tags::extract_cover_art(&path, output_path.as_deref(), &picture_type)
    })
    .await
    .map_err(|error| format!("join error: {error}"))?
    .map_err(|error| error.to_string())
}

async fn group_write_entries(
    entries: Vec<tags::WriteEntry>,
) -> (
    Vec<(PathBuf, Vec<(usize, tags::WriteEntry)>)>,
    Vec<Option<tags::FileWriteResult>>,
) {
    let mut results = (0..entries.len()).map(|_| None).collect::<Vec<_>>();
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
    (groups, results)
}

pub(crate) async fn write_file_tags_workflow<E, F>(
    entries: Vec<tags::WriteEntry>,
    dry_run: bool,
    lock_for: F,
) -> Result<WriteWorkflowOutput, FileWorkflowError<E>>
where
    E: Send + 'static,
    F: Fn(&Path) -> Result<Arc<tokio::sync::Mutex<()>>, E> + Send + Sync + 'static,
{
    if dry_run {
        let semaphore = Arc::new(tokio::sync::Semaphore::new(FILE_CONCURRENCY));
        let mut handles = Vec::with_capacity(entries.len());
        for entry in entries {
            let semaphore = semaphore.clone();
            let path_display = entry.path.display().to_string();
            handles.push(tokio::task::spawn(async move {
                let _permit = semaphore
                    .acquire()
                    .await
                    .expect("semaphore is never closed");
                tokio::task::spawn_blocking(move || tags::write_file_tags_dry_run(&entry))
                    .await
                    .unwrap_or_else(|error| tags::FileDryRunResult::Error {
                        path: path_display,
                        status: "error".to_string(),
                        error: format!("task join error: {error}"),
                    })
            }));
        }
        let mut results = Vec::with_capacity(handles.len());
        let mut previewed = 0;
        let mut failed = 0;
        for handle in handles {
            let result = handle
                .await
                .map_err(|error| FileWorkflowError::Internal(format!("join error: {error}")))?;
            match result {
                tags::FileDryRunResult::Preview { .. } => previewed += 1,
                tags::FileDryRunResult::Error { .. } => failed += 1,
            }
            results.push(result);
        }
        return Ok(WriteWorkflowOutput::DryRun {
            previewed,
            failed,
            results,
        });
    }

    let (groups, mut results) = group_write_entries(entries).await;
    let semaphore = Arc::new(tokio::sync::Semaphore::new(FILE_CONCURRENCY));
    let lock_for = Arc::new(lock_for);
    let mut handles = Vec::with_capacity(groups.len());
    for (canonical_path, entries) in groups {
        let semaphore = semaphore.clone();
        let lock_for = lock_for.clone();
        handles.push(tokio::task::spawn(async move {
            let _permit = semaphore
                .acquire()
                .await
                .expect("semaphore is never closed");
            let mutation_lock = lock_for(&canonical_path).map_err(FileWorkflowError::Lock)?;
            let _guard = mutation_lock.lock().await;
            let mut group_results = Vec::with_capacity(entries.len());
            for (index, mut entry) in entries {
                let path_display = entry.path.display().to_string();
                entry.path = canonical_path.clone();
                let reported_path = path_display.clone();
                let result = tokio::task::spawn_blocking(move || {
                    tags::write_file_tags(&entry).with_reported_path(reported_path)
                })
                .await
                .unwrap_or_else(|error| tags::FileWriteResult::Error {
                    path: path_display,
                    status: "error".to_string(),
                    error: format!("task join error: {error}"),
                });
                group_results.push((index, result));
            }
            Ok::<_, FileWorkflowError<E>>(group_results)
        }));
    }
    for handle in handles {
        let group_results = handle
            .await
            .map_err(|error| FileWorkflowError::Internal(format!("join error: {error}")))??;
        for (index, result) in group_results {
            results[index] = Some(result);
        }
    }
    let results = results
        .into_iter()
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| FileWorkflowError::Internal("missing file tag write result".to_string()))?;
    let mut files_written = 0;
    let mut files_failed = 0;
    let mut fields_written = 0;
    for result in &results {
        match result {
            tags::FileWriteResult::Ok {
                fields_written: fields,
                ..
            } => {
                files_written += 1;
                fields_written += fields.len();
            }
            tags::FileWriteResult::Error { .. } => files_failed += 1,
        }
    }
    Ok(WriteWorkflowOutput::Written {
        files_written,
        files_failed,
        fields_written,
        results,
    })
}

fn image_format(data: &[u8]) -> &'static str {
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
}

pub(crate) async fn embed_cover_art_workflow<E, F>(
    image_path: PathBuf,
    targets: Vec<String>,
    picture_type: String,
    lock_for: F,
) -> Result<EmbedWorkflowOutput, FileWorkflowError<E>>
where
    E: Send + 'static,
    F: Fn(&Path) -> Result<Arc<tokio::sync::Mutex<()>>, E> + Send + Sync + 'static,
{
    let image_size_bytes = tokio::fs::metadata(&image_path)
        .await
        .map(|metadata| metadata.len() as usize)
        .unwrap_or(0);
    let data = tokio::fs::read(&image_path)
        .await
        .map_err(|error| FileWorkflowError::Internal(format!("Failed to read image: {error}")))?;
    let image_format = image_format(&data);

    let mut results = (0..targets.len()).map(|_| None).collect::<Vec<_>>();
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

    let semaphore = Arc::new(tokio::sync::Semaphore::new(FILE_CONCURRENCY));
    let lock_for = Arc::new(lock_for);
    let mut handles = Vec::with_capacity(groups.len());
    for (canonical_path, targets) in groups {
        let semaphore = semaphore.clone();
        let lock_for = lock_for.clone();
        let image_path = image_path.clone();
        let picture_type = picture_type.clone();
        handles.push(tokio::task::spawn(async move {
            let _permit = semaphore
                .acquire()
                .await
                .expect("semaphore is never closed");
            let mutation_lock = lock_for(&canonical_path).map_err(FileWorkflowError::Lock)?;
            let _guard = mutation_lock.lock().await;
            let mut group_results = Vec::with_capacity(targets.len());
            for (index, target) in targets {
                let target_display = target.display().to_string();
                let image_path = image_path.clone();
                let picture_type = picture_type.clone();
                let result = tokio::task::spawn_blocking(move || {
                    tags::embed_cover_art(&image_path, &target, &picture_type)
                })
                .await
                .unwrap_or_else(|error| tags::FileEmbedResult::Error {
                    path: target_display,
                    status: "error".to_string(),
                    error: format!("task join error: {error}"),
                });
                group_results.push((index, result));
            }
            Ok::<_, FileWorkflowError<E>>(group_results)
        }));
    }
    for handle in handles {
        let group_results = handle
            .await
            .map_err(|error| FileWorkflowError::Internal(format!("join error: {error}")))??;
        for (index, result) in group_results {
            results[index] = Some(result);
        }
    }
    let results = results
        .into_iter()
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| FileWorkflowError::Internal("missing cover art embed result".to_string()))?;
    let mut files_embedded = 0;
    let mut files_failed = 0;
    for result in &results {
        match result {
            tags::FileEmbedResult::Ok { .. } => files_embedded += 1,
            tags::FileEmbedResult::Error { .. } => files_failed += 1,
        }
    }
    Ok(EmbedWorkflowOutput {
        files_embedded,
        files_failed,
        image_format,
        image_size_bytes,
        results,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::audio::tags::{CommentMode, WavTarget, WriteEntry};

    fn entry(path: &Path) -> WriteEntry {
        WriteEntry {
            path: path.to_path_buf(),
            tags: HashMap::new(),
            wav_targets: vec![WavTarget::Id3v2, WavTarget::RiffInfo],
            comment_mode: CommentMode::Replace,
        }
    }

    #[tokio::test]
    async fn file_tag_workflow_preserves_dry_run() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(temp.path(), b"unchanged").unwrap();
        let before = std::fs::read(temp.path()).unwrap();
        let output = write_file_tags_workflow(vec![entry(temp.path())], true, |_| {
            Ok::<_, ()>(Arc::new(tokio::sync::Mutex::new(())))
        })
        .await
        .unwrap();
        assert!(matches!(output, WriteWorkflowOutput::DryRun { .. }));
        assert_eq!(std::fs::read(temp.path()).unwrap(), before);
    }

    #[tokio::test]
    async fn boundary_file_tag_workflow_serializes_canonical_paths() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        let (groups, results) =
            group_write_entries(vec![entry(temp.path()), entry(temp.path())]).await;
        assert_eq!(groups.len(), 1);
        assert_eq!(
            groups[0]
                .1
                .iter()
                .map(|(index, _)| *index)
                .collect::<Vec<_>>(),
            vec![0, 1]
        );
        assert!(results.iter().all(Option::is_none));
    }
}
