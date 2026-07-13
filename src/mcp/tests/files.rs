use crate::mcp::error::mcp_internal_error;
use crate::mcp::files::{
    EmbedCoverArtParams, ExtractCoverArtParams, WriteFileTagsEntry, WriteFileTagsParams,
};
use crate::mcp::server::ReklawdboxServer;
use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use rmcp::ErrorData as McpError;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;

use super::common::extract_json;

const AUDIO_FILE_MUTATION_TIMEOUT: Duration = Duration::from_secs(5);

type AudioFileMutationTaskOutput = Result<CallToolResult, McpError>;

struct AudioFileMutationTaskCleanup {
    handles: Vec<Option<tokio::task::JoinHandle<AudioFileMutationTaskOutput>>>,
}

impl AudioFileMutationTaskCleanup {
    fn new() -> Self {
        Self {
            handles: Vec::new(),
        }
    }

    fn push(&mut self, handle: tokio::task::JoinHandle<AudioFileMutationTaskOutput>) {
        self.handles.push(Some(handle));
    }

    fn all_pending(&self) -> bool {
        self.handles
            .iter()
            .flatten()
            .all(|handle| !handle.is_finished())
    }

    async fn join(
        &mut self,
        index: usize,
        phase: &str,
    ) -> Result<AudioFileMutationTaskOutput, String> {
        let outcome = {
            let handle = self
                .handles
                .get_mut(index)
                .and_then(Option::as_mut)
                .ok_or_else(|| format!("{phase}: task handle is missing"))?;

            match tokio::time::timeout(AUDIO_FILE_MUTATION_TIMEOUT, &mut *handle).await {
                Ok(Ok(output)) => Ok(output),
                Ok(Err(err)) => Err(format!("{phase}: task join failed: {err}")),
                Err(_) => {
                    handle.abort();
                    let cleanup =
                        tokio::time::timeout(AUDIO_FILE_MUTATION_TIMEOUT, &mut *handle).await;
                    if cleanup.is_err() {
                        return Err(format!(
                            "{phase}: task timed out and abort cleanup did not finish within five seconds"
                        ));
                    }
                    Err(format!("{phase}: task did not finish within five seconds"))
                }
            }
        };
        self.handles[index].take();
        outcome
    }

    async fn abort_all(&mut self) -> Result<(), String> {
        for handle in self.handles.iter().flatten() {
            handle.abort();
        }

        for (index, slot) in self.handles.iter_mut().enumerate() {
            let Some(mut handle) = slot.take() else {
                continue;
            };
            if tokio::time::timeout(AUDIO_FILE_MUTATION_TIMEOUT, &mut handle)
                .await
                .is_err()
            {
                return Err(format!(
                    "audio mutation task {index} did not join during cleanup within five seconds"
                ));
            }
        }
        Ok(())
    }
}

impl Drop for AudioFileMutationTaskCleanup {
    fn drop(&mut self) {
        for handle in self.handles.iter().flatten() {
            handle.abort();
        }
    }
}

fn write_audio_file_mutation_wav(path: &std::path::Path) {
    let data_size: u32 = 2;
    let file_size = 36 + data_size;
    let mut header = Vec::new();
    header.extend_from_slice(b"RIFF");
    header.extend_from_slice(&file_size.to_le_bytes());
    header.extend_from_slice(b"WAVE");
    header.extend_from_slice(b"fmt ");
    header.extend_from_slice(&16u32.to_le_bytes());
    header.extend_from_slice(&1u16.to_le_bytes());
    header.extend_from_slice(&1u16.to_le_bytes());
    header.extend_from_slice(&44_100u32.to_le_bytes());
    header.extend_from_slice(&88_200u32.to_le_bytes());
    header.extend_from_slice(&2u16.to_le_bytes());
    header.extend_from_slice(&16u16.to_le_bytes());
    header.extend_from_slice(b"data");
    header.extend_from_slice(&data_size.to_le_bytes());
    header.extend_from_slice(&[0u8; 2]);
    std::fs::write(path, header).expect("synthetic WAV should write");
}

fn seed_audio_file_wav_layer(
    path: &std::path::Path,
    target: crate::adapters::audio::tags::WavTarget,
    tags: HashMap<String, Option<String>>,
) {
    let result =
        crate::adapters::audio::tags::write_file_tags(&crate::adapters::audio::tags::WriteEntry {
            path: path.to_path_buf(),
            tags,
            wav_targets: vec![target],
            comment_mode: crate::adapters::audio::tags::CommentMode::Replace,
        });
    assert!(
        matches!(
            result,
            crate::adapters::audio::tags::FileWriteResult::Ok { .. }
        ),
        "synthetic WAV layer should seed successfully: {result:?}"
    );
}

fn write_audio_file_mutation_png(path: &std::path::Path) {
    let png = [
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 4,
        0, 0, 0, 181, 28, 12, 2, 0, 0, 0, 11, 73, 68, 65, 84, 120, 218, 99, 100, 248, 15, 0, 1, 5,
        1, 1, 39, 24, 227, 102, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
    ];
    std::fs::write(path, png).expect("synthetic PNG should write");
}

fn audio_file_mutation_write_entry(
    path: &std::path::Path,
    comment: &str,
    comment_mode: crate::adapters::audio::tags::CommentMode,
) -> WriteFileTagsEntry {
    WriteFileTagsEntry {
        path: path.display().to_string(),
        tags: HashMap::from([("comment".to_string(), Some(comment.to_string()))]),
        wav_targets: Some(vec![crate::adapters::audio::tags::WavTarget::Id3v2]),
        comment_mode: Some(comment_mode),
    }
}

fn audio_file_mutation_state(path: &std::path::Path) -> Result<(Option<String>, bool), String> {
    let fields = ["comment".to_string()];
    match crate::adapters::audio::tags::read_file_tags(path, Some(&fields), true) {
        crate::adapters::audio::tags::FileReadResult::Wav {
            id3v2, cover_art, ..
        } => Ok((id3v2.get("comment").cloned().flatten(), cover_art.is_some())),
        crate::adapters::audio::tags::FileReadResult::Single {
            tags, cover_art, ..
        } => Ok((tags.get("comment").cloned().flatten(), cover_art.is_some())),
        crate::adapters::audio::tags::FileReadResult::Error { error, .. } => Err(error),
    }
}

fn assert_cover_art_invalid_params(error: &McpError, rejected: &str) {
    assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    assert!(error.message.contains(&format!("{rejected:?}")));
    assert!(error.message.contains("front_cover"));
    assert!(error.message.contains("back_cover"));
}

fn cover_art_picture_type_schema_description<T: schemars::JsonSchema>() -> String {
    let schema = schemars::schema_for!(T);
    schema
        .as_value()
        .pointer("/properties/picture_type/description")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_else(|| panic!("{} picture_type description is missing", T::schema_name()))
        .to_string()
}

fn cover_art_accepted_picture_type_tokens(description: &str) -> Vec<&str> {
    let (_, accepted) = description
        .split_once("Accepted exact values:")
        .expect("schema should introduce accepted picture types");
    let (accepted, _) = accepted
        .split_once(". Unknown values are rejected")
        .expect("schema should terminate the accepted picture type list");
    accepted
        .split(|character: char| !(character.is_ascii_alphanumeric() || character == '_'))
        .filter(|token| !token.is_empty())
        .collect()
}

async fn wait_at_audio_file_mutation_barrier(
    barrier: &tokio::sync::Barrier,
    phase: &str,
) -> Result<(), String> {
    tokio::time::timeout(AUDIO_FILE_MUTATION_TIMEOUT, barrier.wait())
        .await
        .map(|_| ())
        .map_err(|_| format!("{phase}: barrier did not release within five seconds"))
}

async fn wait_for_audio_file_mutation_strong_count(
    server: &ReklawdboxServer,
    canonical_path: &std::path::Path,
    minimum: usize,
    phase: &str,
) -> Result<(), String> {
    let wait = async {
        loop {
            let strong_count = {
                let locks = server
                    .context
                    .mutation
                    .audio_file_mutation_locks
                    .lock()
                    .map_err(|_| "audio file mutation registry poisoned".to_string())?;
                locks
                    .get(canonical_path)
                    .map_or(0, std::sync::Weak::strong_count)
            };
            if strong_count >= minimum {
                return Ok(());
            }
            tokio::task::yield_now().await;
        }
    };

    tokio::time::timeout(AUDIO_FILE_MUTATION_TIMEOUT, wait)
        .await
        .map_err(|_| format!("{phase}: lock waiters did not arrive within five seconds"))?
}

#[tokio::test]
async fn dry_run_response_reports_wav_layers_without_mutation() {
    let temp_dir = tempfile::tempdir().expect("temp audio directory should create");
    let audio_path = temp_dir.path().join("dry-run.wav");
    write_audio_file_mutation_wav(&audio_path);
    seed_audio_file_wav_layer(
        &audio_path,
        crate::adapters::audio::tags::WavTarget::Id3v2,
        HashMap::from([("artist".to_string(), Some("ID3 old".to_string()))]),
    );
    seed_audio_file_wav_layer(
        &audio_path,
        crate::adapters::audio::tags::WavTarget::RiffInfo,
        HashMap::from([("artist".to_string(), Some("RIFF old".to_string()))]),
    );
    let fields = ["artist".to_string()];
    let before_tags = serde_json::to_value(crate::adapters::audio::tags::read_file_tags(
        &audio_path,
        Some(&fields),
        false,
    ))
    .expect("pre-dry-run tags should serialize");
    let before_bytes = std::fs::read(&audio_path).expect("synthetic WAV should read");

    let server = ReklawdboxServer::new(None);
    let output = server
        .write_file_tags(Parameters(WriteFileTagsParams {
            writes: vec![WriteFileTagsEntry {
                path: audio_path.display().to_string(),
                tags: HashMap::from([("artist".to_string(), Some("new".to_string()))]),
                wav_targets: None,
                comment_mode: None,
            }],
            dry_run: Some(true),
        }))
        .await
        .expect("MCP WAV dry-run should succeed");
    let payload = extract_json(&output);
    let preview = &payload["results"][0];

    assert_eq!(payload["dry_run"], true);
    assert_eq!(payload["summary"]["files_previewed"], 1);
    assert_eq!(payload["summary"]["files_failed"], 0);
    assert_eq!(
        preview["changes_by_layer"]["id3v2"]["artist"]["old"],
        "ID3 old"
    );
    assert_eq!(
        preview["changes_by_layer"]["riff_info"]["artist"]["old"],
        "RIFF old"
    );
    assert_eq!(preview["changes"], preview["changes_by_layer"]["id3v2"]);
    assert_eq!(
        preview["wav_targets"],
        serde_json::json!(["id3v2", "riff_info"])
    );

    let after_tags = serde_json::to_value(crate::adapters::audio::tags::read_file_tags(
        &audio_path,
        Some(&fields),
        false,
    ))
    .expect("post-dry-run tags should serialize");
    assert_eq!(after_tags, before_tags);
    assert_eq!(
        std::fs::read(&audio_path).expect("synthetic WAV should still read"),
        before_bytes
    );
}

#[tokio::test]
async fn cover_art_invalid_picture_type_extract_is_invalid_params_before_io() {
    let temp_dir = tempfile::tempdir().expect("temp directory should create");
    let output_path = temp_dir.path().join("should-not-exist.png");
    let server = ReklawdboxServer::new(None);

    let error = server
        .extract_cover_art(Parameters(ExtractCoverArtParams {
            path: temp_dir.path().join("missing.wav").display().to_string(),
            output_path: Some(output_path.display().to_string()),
            picture_type: Some("garbage".to_string()),
        }))
        .await
        .unwrap_err();

    assert_cover_art_invalid_params(&error, "garbage");
    assert!(!output_path.exists());
}

#[tokio::test]
async fn cover_art_invalid_picture_type_embed_starts_no_file_work() {
    let temp_dir = tempfile::tempdir().expect("temp directory should create");
    let audio_path = temp_dir.path().join("unchanged.wav");
    let missing_target = temp_dir.path().join("missing.wav");
    let missing_image = temp_dir.path().join("missing.png");
    write_audio_file_mutation_wav(&audio_path);
    let original_audio = std::fs::read(&audio_path).expect("synthetic WAV should read");
    let server = ReklawdboxServer::new(None);

    let error = server
        .embed_cover_art(Parameters(EmbedCoverArtParams {
            image_path: missing_image.display().to_string(),
            target_audio_files: vec![
                audio_path.display().to_string(),
                missing_target.display().to_string(),
            ],
            picture_type: Some("Front_Cover".to_string()),
        }))
        .await
        .unwrap_err();

    assert_cover_art_invalid_params(&error, "Front_Cover");
    assert_eq!(
        std::fs::read(&audio_path).expect("synthetic WAV should remain readable"),
        original_audio
    );
    assert_eq!(server.audio_file_mutation_registry_len().unwrap(), 0);
}

#[tokio::test]
async fn cover_art_valid_aliases_work_through_mcp_handlers() {
    let temp_dir = tempfile::tempdir().expect("temp directory should create");
    let audio_path = temp_dir.path().join("alias.wav");
    let image_path = temp_dir.path().join("alias.png");
    let output_path = temp_dir.path().join("alias-extracted.png");
    write_audio_file_mutation_wav(&audio_path);
    write_audio_file_mutation_png(&image_path);
    let server = ReklawdboxServer::new(None);

    let embed = server
        .embed_cover_art(Parameters(EmbedCoverArtParams {
            image_path: image_path.display().to_string(),
            target_audio_files: vec![audio_path.display().to_string()],
            picture_type: Some("cover_front".to_string()),
        }))
        .await
        .expect("cover_front embed alias should succeed");
    assert_eq!(extract_json(&embed)["summary"]["files_embedded"], 1);

    let extract = server
        .extract_cover_art(Parameters(ExtractCoverArtParams {
            path: audio_path.display().to_string(),
            output_path: Some(output_path.display().to_string()),
            picture_type: Some("cover_front".to_string()),
        }))
        .await
        .expect("cover_front extract alias should succeed");
    let payload = extract_json(&extract);
    assert_eq!(payload["picture_type"], "front_cover");
    assert_eq!(
        std::fs::read(output_path).expect("extracted art should read"),
        std::fs::read(image_path).expect("source art should read")
    );
}

#[test]
fn cover_art_picture_type_schema_descriptions_match_parser_contract() {
    for (surface, description) in [
        (
            "extract_cover_art",
            cover_art_picture_type_schema_description::<ExtractCoverArtParams>(),
        ),
        (
            "embed_cover_art",
            cover_art_picture_type_schema_description::<EmbedCoverArtParams>(),
        ),
    ] {
        assert_eq!(
            cover_art_accepted_picture_type_tokens(&description),
            crate::adapters::audio::tags::ACCEPTED_PICTURE_TYPES,
            "{surface} schema picture types drifted: {description}"
        );
        assert!(description.contains("Unknown values are rejected"));
    }
}

#[cfg(unix)]
#[tokio::test]
async fn audio_file_mutation_canonical_aliases_share_and_serialize_lock() {
    let cwd = std::env::current_dir().expect("current directory should resolve");
    let temp_dir = tempfile::Builder::new()
        .prefix("audio-file-mutation-alias-")
        .tempdir_in(&cwd)
        .expect("temp audio directory should create under the working directory");
    let audio_path = temp_dir.path().join("identity.wav");
    write_audio_file_mutation_wav(&audio_path);
    let absolute_path = std::fs::canonicalize(&audio_path).expect("audio path should canonicalize");
    let relative_path = absolute_path
        .strip_prefix(&cwd)
        .expect("temp audio path should be relative to the working directory")
        .to_path_buf();
    let symlink_path = temp_dir.path().join("identity-alias.wav");
    std::os::unix::fs::symlink(&absolute_path, &symlink_path).expect("audio symlink should create");

    let canonical_absolute = tokio::time::timeout(
        AUDIO_FILE_MUTATION_TIMEOUT,
        tokio::fs::canonicalize(&absolute_path),
    )
    .await
    .expect("absolute canonicalization should finish within five seconds")
    .expect("absolute audio path should canonicalize");
    let canonical_relative = tokio::time::timeout(
        AUDIO_FILE_MUTATION_TIMEOUT,
        tokio::fs::canonicalize(&relative_path),
    )
    .await
    .expect("relative canonicalization should finish within five seconds")
    .expect("relative audio path should canonicalize");
    let canonical_symlink = tokio::time::timeout(
        AUDIO_FILE_MUTATION_TIMEOUT,
        tokio::fs::canonicalize(&symlink_path),
    )
    .await
    .expect("symlink canonicalization should finish within five seconds")
    .expect("symlink audio path should canonicalize");
    assert_eq!(canonical_absolute, canonical_relative);
    assert_eq!(canonical_absolute, canonical_symlink);

    let server = ReklawdboxServer::new(None);
    let absolute_lock = server
        .audio_file_mutation_lock(&canonical_absolute)
        .expect("absolute mutation lock should resolve");
    let relative_lock = server
        .audio_file_mutation_lock(&canonical_relative)
        .expect("relative mutation lock should resolve");
    let symlink_lock = server
        .audio_file_mutation_lock(&canonical_symlink)
        .expect("symlink mutation lock should resolve");
    assert!(Arc::ptr_eq(&absolute_lock, &relative_lock));
    assert!(Arc::ptr_eq(&absolute_lock, &symlink_lock));

    let held_guard = tokio::time::timeout(AUDIO_FILE_MUTATION_TIMEOUT, absolute_lock.lock())
        .await
        .expect("canonical lock should be acquired within five seconds");
    let mut alias_waiter = Box::pin(symlink_lock.lock());
    std::future::poll_fn(|cx| match alias_waiter.as_mut().poll(cx) {
        std::task::Poll::Pending => std::task::Poll::Ready(()),
        std::task::Poll::Ready(_) => {
            panic!("alias lock acquired while the canonical identity was held")
        }
    })
    .await;
    drop(held_guard);
    let _alias_guard = tokio::time::timeout(AUDIO_FILE_MUTATION_TIMEOUT, &mut alias_waiter)
        .await
        .expect("alias lock should acquire after release within five seconds");
}

#[cfg(unix)]
#[tokio::test]
async fn audio_file_mutation_duplicate_alias_writes_preserve_input_order() {
    let temp_dir = tempfile::tempdir().expect("temp audio directory should create");
    let audio_path = temp_dir.path().join("ordered.wav");
    let symlink_path = temp_dir.path().join("ordered-alias.wav");
    write_audio_file_mutation_wav(&audio_path);
    std::os::unix::fs::symlink(&audio_path, &symlink_path).expect("audio symlink should create");

    let seed = crate::adapters::audio::tags::WriteEntry {
        path: audio_path.clone(),
        tags: HashMap::from([("comment".to_string(), Some("base".to_string()))]),
        wav_targets: vec![crate::adapters::audio::tags::WavTarget::Id3v2],
        comment_mode: crate::adapters::audio::tags::CommentMode::Replace,
    };
    assert!(matches!(
        crate::adapters::audio::tags::write_file_tags(&seed),
        crate::adapters::audio::tags::FileWriteResult::Ok { .. }
    ));

    let server = ReklawdboxServer::new(None);
    let params = WriteFileTagsParams {
        writes: vec![
            audio_file_mutation_write_entry(
                &audio_path,
                "before",
                crate::adapters::audio::tags::CommentMode::Prepend,
            ),
            audio_file_mutation_write_entry(
                &symlink_path,
                "after",
                crate::adapters::audio::tags::CommentMode::Append,
            ),
        ],
        dry_run: Some(false),
    };
    let mut tasks = AudioFileMutationTaskCleanup::new();
    tasks.push(tokio::spawn(async move {
        server.write_file_tags(Parameters(params)).await
    }));
    let output = match tasks.join(0, "duplicate alias write").await {
        Ok(Ok(output)) => output,
        Ok(Err(err)) => panic!("duplicate alias write returned an MCP error: {err:?}"),
        Err(err) => panic!("duplicate alias write failed: {err}"),
    };

    let payload = extract_json(&output);
    assert_eq!(payload["summary"]["files_written"], 2);
    assert_eq!(payload["summary"]["files_failed"], 0);
    assert_eq!(
        payload["results"][0]["path"].as_str(),
        Some(audio_path.to_string_lossy().as_ref())
    );
    assert_eq!(
        payload["results"][1]["path"].as_str(),
        Some(symlink_path.to_string_lossy().as_ref())
    );
    let (comment, _) =
        audio_file_mutation_state(&audio_path).expect("ordered audio tags should remain readable");
    assert_eq!(comment.as_deref(), Some("before | base | after"));
}

#[cfg(unix)]
#[tokio::test]
async fn audio_file_mutation_dual_layer_wav_preserves_symlink_and_updates_target() {
    let temp_dir = tempfile::tempdir().expect("temp audio directory should create");
    let audio_path = temp_dir.path().join("target.wav");
    let symlink_path = temp_dir.path().join("alias.wav");
    write_audio_file_mutation_wav(&audio_path);
    std::os::unix::fs::symlink(&audio_path, &symlink_path).expect("audio symlink should create");

    let server = ReklawdboxServer::new(None);
    let output = server
        .write_file_tags(Parameters(WriteFileTagsParams {
            writes: vec![WriteFileTagsEntry {
                path: symlink_path.to_string_lossy().to_string(),
                tags: HashMap::from([("comment".to_string(), Some("through-link".to_string()))]),
                wav_targets: None,
                comment_mode: None,
            }],
            dry_run: Some(false),
        }))
        .await
        .expect("dual-layer symlink write should succeed");

    let payload = extract_json(&output);
    assert_eq!(payload["summary"]["files_written"], 1);
    assert!(
        std::fs::symlink_metadata(&symlink_path)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    let (comment, _) =
        audio_file_mutation_state(&audio_path).expect("target tags should remain readable");
    assert_eq!(comment.as_deref(), Some("through-link"));
}

#[tokio::test]
async fn audio_file_mutation_tag_and_art_requests_share_one_lock() {
    let temp_dir = tempfile::tempdir().expect("temp audio directory should create");
    let audio_path = temp_dir.path().join("shared.wav");
    let image_path = temp_dir.path().join("cover.png");
    write_audio_file_mutation_wav(&audio_path);
    write_audio_file_mutation_png(&image_path);
    let canonical_path =
        std::fs::canonicalize(&audio_path).expect("audio path should canonicalize");

    let server = ReklawdboxServer::new(None);
    let shared_lock = server
        .audio_file_mutation_lock(&canonical_path)
        .expect("shared mutation lock should resolve");
    let mut held_guard = Some(
        tokio::time::timeout(AUDIO_FILE_MUTATION_TIMEOUT, shared_lock.lock())
            .await
            .expect("shared lock should acquire within five seconds"),
    );
    let barrier = Arc::new(tokio::sync::Barrier::new(3));
    let mut tasks = AudioFileMutationTaskCleanup::new();

    {
        let server = server.clone();
        let barrier = Arc::clone(&barrier);
        let params = WriteFileTagsParams {
            writes: vec![audio_file_mutation_write_entry(
                &audio_path,
                "tag-update",
                crate::adapters::audio::tags::CommentMode::Replace,
            )],
            dry_run: Some(false),
        };
        tasks.push(tokio::spawn(async move {
            tokio::time::timeout(AUDIO_FILE_MUTATION_TIMEOUT, barrier.wait())
                .await
                .map_err(|_| mcp_internal_error("tag request barrier timed out"))?;
            server.write_file_tags(Parameters(params)).await
        }));
    }
    {
        let server = server.clone();
        let barrier = Arc::clone(&barrier);
        let params = EmbedCoverArtParams {
            image_path: image_path.display().to_string(),
            target_audio_files: vec![audio_path.display().to_string()],
            picture_type: Some("front_cover".to_string()),
        };
        tasks.push(tokio::spawn(async move {
            tokio::time::timeout(AUDIO_FILE_MUTATION_TIMEOUT, barrier.wait())
                .await
                .map_err(|_| mcp_internal_error("art request barrier timed out"))?;
            server.embed_cover_art(Parameters(params)).await
        }));
    }

    let scenario = async {
        wait_at_audio_file_mutation_barrier(&barrier, "shared request start").await?;
        wait_for_audio_file_mutation_strong_count(
            &server,
            &canonical_path,
            3,
            "shared tag and art waiters",
        )
        .await?;
        if !tasks.all_pending() {
            return Err("tag and art requests should both wait on the held lock".to_string());
        }

        drop(held_guard.take());
        let tag_output = tasks
            .join(0, "shared tag request")
            .await?
            .map_err(|err| format!("shared tag request returned an MCP error: {err:?}"))?;
        let art_output = tasks
            .join(1, "shared art request")
            .await?
            .map_err(|err| format!("shared art request returned an MCP error: {err:?}"))?;
        Ok::<_, String>((tag_output, art_output))
    };

    let scenario_result = tokio::time::timeout(AUDIO_FILE_MUTATION_TIMEOUT, scenario).await;
    let (tag_output, art_output) = match scenario_result {
        Ok(Ok(outputs)) => outputs,
        Ok(Err(err)) => {
            drop(held_guard.take());
            let cleanup = tasks.abort_all().await;
            panic!("shared tag/art scenario failed: {err}; cleanup: {cleanup:?}");
        }
        Err(_) => {
            drop(held_guard.take());
            let cleanup = tasks.abort_all().await;
            panic!("shared tag/art scenario timed out; cleanup: {cleanup:?}");
        }
    };

    assert_eq!(extract_json(&tag_output)["summary"]["files_written"], 1);
    assert_eq!(extract_json(&art_output)["summary"]["files_embedded"], 1);
    let (comment, has_cover_art) =
        audio_file_mutation_state(&audio_path).expect("shared audio tags should remain readable");
    assert_eq!(comment.as_deref(), Some("tag-update"));
    assert!(has_cover_art, "cover art should survive the tag mutation");
}

#[tokio::test]
async fn audio_file_mutation_different_file_progress_is_independent() {
    let temp_dir = tempfile::tempdir().expect("temp audio directory should create");
    let first_path = temp_dir.path().join("first.wav");
    let second_path = temp_dir.path().join("second.wav");
    write_audio_file_mutation_wav(&first_path);
    write_audio_file_mutation_wav(&second_path);
    let canonical_first =
        std::fs::canonicalize(&first_path).expect("first audio path should canonicalize");

    let server = ReklawdboxServer::new(None);
    let first_lock = server
        .audio_file_mutation_lock(&canonical_first)
        .expect("first mutation lock should resolve");
    let mut held_guard = Some(
        tokio::time::timeout(AUDIO_FILE_MUTATION_TIMEOUT, first_lock.lock())
            .await
            .expect("first lock should acquire within five seconds"),
    );
    let barrier = Arc::new(tokio::sync::Barrier::new(3));
    let mut tasks = AudioFileMutationTaskCleanup::new();

    for (path, comment, barrier_error) in [
        (
            first_path.clone(),
            "first-update",
            "first request barrier timed out",
        ),
        (
            second_path.clone(),
            "second-update",
            "second request barrier timed out",
        ),
    ] {
        let server = server.clone();
        let barrier = Arc::clone(&barrier);
        let params = WriteFileTagsParams {
            writes: vec![audio_file_mutation_write_entry(
                &path,
                comment,
                crate::adapters::audio::tags::CommentMode::Replace,
            )],
            dry_run: Some(false),
        };
        tasks.push(tokio::spawn(async move {
            tokio::time::timeout(AUDIO_FILE_MUTATION_TIMEOUT, barrier.wait())
                .await
                .map_err(|_| mcp_internal_error(barrier_error))?;
            server.write_file_tags(Parameters(params)).await
        }));
    }

    let scenario = async {
        wait_at_audio_file_mutation_barrier(&barrier, "independent request start").await?;
        wait_for_audio_file_mutation_strong_count(
            &server,
            &canonical_first,
            2,
            "first file waiter",
        )
        .await?;

        let second_output = tasks
            .join(1, "independent second-file request")
            .await?
            .map_err(|err| format!("second-file request returned an MCP error: {err:?}"))?;
        if !tasks.all_pending() {
            return Err(
                "first-file request should remain blocked after second completes".to_string(),
            );
        }
        let (second_comment, _) = audio_file_mutation_state(&second_path)
            .map_err(|err| format!("second file became unreadable: {err}"))?;
        if second_comment.as_deref() != Some("second-update") {
            return Err(format!(
                "second file did not complete independently: {second_comment:?}"
            ));
        }

        drop(held_guard.take());
        let first_output = tasks
            .join(0, "released first-file request")
            .await?
            .map_err(|err| format!("first-file request returned an MCP error: {err:?}"))?;
        Ok::<_, String>((first_output, second_output))
    };

    let scenario_result = tokio::time::timeout(AUDIO_FILE_MUTATION_TIMEOUT, scenario).await;
    let (first_output, second_output) = match scenario_result {
        Ok(Ok(outputs)) => outputs,
        Ok(Err(err)) => {
            drop(held_guard.take());
            let cleanup = tasks.abort_all().await;
            panic!("independent-file scenario failed: {err}; cleanup: {cleanup:?}");
        }
        Err(_) => {
            drop(held_guard.take());
            let cleanup = tasks.abort_all().await;
            panic!("independent-file scenario timed out; cleanup: {cleanup:?}");
        }
    };

    assert_eq!(extract_json(&first_output)["summary"]["files_written"], 1);
    assert_eq!(extract_json(&second_output)["summary"]["files_written"], 1);
    let (first_comment, _) =
        audio_file_mutation_state(&first_path).expect("first audio tags should remain readable");
    assert_eq!(first_comment.as_deref(), Some("first-update"));
}

#[test]
fn audio_file_mutation_registry_prunes_dead_weak_entries() {
    let temp_dir = tempfile::tempdir().expect("temp audio directory should create");
    let first_path = temp_dir.path().join("first.wav");
    let second_path = temp_dir.path().join("second.wav");
    write_audio_file_mutation_wav(&first_path);
    write_audio_file_mutation_wav(&second_path);
    let canonical_first =
        std::fs::canonicalize(&first_path).expect("first audio path should canonicalize");
    let canonical_second =
        std::fs::canonicalize(&second_path).expect("second audio path should canonicalize");
    let server = ReklawdboxServer::new(None);

    let first_lock = server
        .audio_file_mutation_lock(&canonical_first)
        .expect("first mutation lock should resolve");
    assert_eq!(
        server
            .audio_file_mutation_registry_len()
            .expect("registry length should read"),
        1
    );
    drop(first_lock);

    let second_lock = server
        .audio_file_mutation_lock(&canonical_second)
        .expect("second mutation lock should resolve");
    assert_eq!(
        server
            .audio_file_mutation_registry_len()
            .expect("registry length should read after cleanup"),
        1,
        "requesting a new identity should remove the stale first Weak entry"
    );
    drop(second_lock);

    let _replacement_lock = server
        .audio_file_mutation_lock(&canonical_first)
        .expect("replacement mutation lock should resolve");
    assert_eq!(
        server
            .audio_file_mutation_registry_len()
            .expect("bounded registry length should read"),
        1
    );
}
