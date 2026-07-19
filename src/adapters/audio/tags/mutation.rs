//! Validated layer planning, tag mutation, and atomic WAV replacement.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use lofty::config::WriteOptions;
use lofty::file::{FileType, TaggedFileExt};
use lofty::prelude::TagExt;
use lofty::probe::Probe;
use lofty::tag::{Tag, TagType};

use super::fields::{TagEdit, TagField, ValidatedTagPatch, get_tag_field, merge_comment};
use super::model::{
    CommentMode, DryRunChange, FileDryRunResult, FileWriteResult, TagError, WavTarget, WriteEntry,
};
use super::read::parse_options;

impl From<&WavTarget> for TagType {
    fn from(target: &WavTarget) -> Self {
        match target {
            WavTarget::Id3v2 => TagType::Id3v2,
            WavTarget::RiffInfo => TagType::RiffInfo,
        }
    }
}

impl WavTarget {
    pub(super) const fn capability(&self) -> LayerCapability {
        match self {
            Self::Id3v2 => LayerCapability::Id3v2,
            Self::RiffInfo => LayerCapability::RiffInfo,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LayerCapability {
    Id3v2,
    RiffInfo,
    Primary,
}

impl LayerCapability {
    pub(super) const fn supports(self, field: TagField) -> bool {
        !matches!(self, Self::RiffInfo) || field.is_riff_info()
    }
}

#[derive(Debug)]
pub(super) struct ExistingLayerValues {
    values: BTreeMap<TagField, Option<String>>,
}

impl ExistingLayerValues {
    pub(super) fn read(tag: Option<&Tag>, patch: &ValidatedTagPatch) -> Self {
        let values = patch
            .edits
            .keys()
            .copied()
            .map(|field| (field, tag.and_then(|tag| get_tag_field(tag, field))))
            .collect();
        Self { values }
    }

    fn get(&self, field: TagField) -> Option<String> {
        self.values.get(&field).cloned().flatten()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PlannedTagMutation {
    field: TagField,
    edit: TagEdit,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct LayerMutationPlan {
    operations: Vec<PlannedTagMutation>,
    changes: BTreeMap<String, DryRunChange>,
}

pub(super) fn plan_layer_mutation(
    patch: &ValidatedTagPatch,
    capability: LayerCapability,
    existing: &ExistingLayerValues,
    comment_mode: CommentMode,
) -> LayerMutationPlan {
    let mut operations = Vec::new();
    let mut changes = BTreeMap::new();

    for (&field, edit) in &patch.edits {
        if !capability.supports(field) {
            continue;
        }

        let old = existing.get(field);
        let effective_edit = match edit {
            TagEdit::Delete => TagEdit::Delete,
            TagEdit::Set(value)
                if field == TagField::Comment && comment_mode != CommentMode::Replace =>
            {
                TagEdit::Set(merge_comment(value, old.as_deref(), comment_mode))
            }
            TagEdit::Set(value) => TagEdit::Set(value.clone()),
        };
        let new = match &effective_edit {
            TagEdit::Set(value) => Some(value.clone()),
            TagEdit::Delete => None,
        };

        if old == new {
            continue;
        }

        changes.insert(
            field.as_str().to_string(),
            DryRunChange {
                old,
                new: new.clone(),
            },
        );
        operations.push(PlannedTagMutation {
            field,
            edit: effective_edit,
        });
    }

    LayerMutationPlan {
        operations,
        changes,
    }
}

// ---------------------------------------------------------------------------
// 2. write_file_tags
// ---------------------------------------------------------------------------

/// Write tags to a single audio file with merge semantics.
///
/// Read-modify-write: only the specified fields are touched, everything else
/// is preserved. Both `None` and `Some("")` delete the tag frame.
pub fn write_file_tags(entry: &WriteEntry) -> FileWriteResult {
    let path_str = entry.path.display().to_string();

    let patch = match ValidatedTagPatch::try_from(&entry.tags) {
        Ok(patch) => patch,
        Err(error) => {
            return FileWriteResult::Error {
                path: path_str,
                status: "error".to_string(),
                error: error.to_string(),
            };
        }
    };

    match write_file_tags_inner(entry, &patch) {
        Ok(result) => result,
        Err(e) => FileWriteResult::Error {
            path: path_str,
            status: "error".to_string(),
            error: e.to_string(),
        },
    }
}

#[cfg(test)]
thread_local! {
    static TEST_FAIL_TAG_LAYER_WRITE: std::cell::Cell<Option<TagType>> = const {
        std::cell::Cell::new(None)
    };
}

#[cfg(test)]
pub(super) fn with_test_tag_layer_write_failure<T>(
    tag_type: TagType,
    operation: impl FnOnce() -> T,
) -> T {
    struct ResetFailure;

    impl Drop for ResetFailure {
        fn drop(&mut self) {
            TEST_FAIL_TAG_LAYER_WRITE.with(|failure| failure.set(None));
        }
    }

    TEST_FAIL_TAG_LAYER_WRITE.with(|failure| {
        assert!(
            failure.replace(Some(tag_type)).is_none(),
            "test tag-layer failure must not already be armed"
        );
    });
    let _reset = ResetFailure;
    operation()
}

/// Generate a temp path in the same directory as the original for atomic rename.
/// Format: `.{stem}.rklw-{pid}-{ms}.{ext}`
fn atomic_temp_path(original: &Path) -> PathBuf {
    let stem = original
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("file");
    let ext = original
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("tmp");
    let pid = std::process::id();
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let filename = format!(".{stem}.rklw-{pid}-{ms}.{ext}");
    original.with_file_name(filename)
}

fn write_file_tags_inner(
    entry: &WriteEntry,
    patch: &ValidatedTagPatch,
) -> Result<FileWriteResult, TagError> {
    let path = &entry.path;
    let path_str = path.display().to_string();

    let tagged_file = Probe::open(path)
        .map_err(|e| TagError::Io(format!("Failed to open: {e}")))?
        .options(parse_options(false))
        .read()
        .map_err(|e| TagError::Io(format!("Failed to read: {e}")))?;

    let file_type = tagged_file.file_type();
    let is_wav = file_type == FileType::Wav;

    let wav_targets = if is_wav {
        if entry.wav_targets.is_empty() {
            vec![WavTarget::Id3v2, WavTarget::RiffInfo]
        } else {
            entry.wav_targets.clone()
        }
    } else {
        vec![]
    };

    let mut fields_written = Vec::new();
    let mut fields_deleted = Vec::new();

    if is_wav && wav_targets.len() > 1 {
        // Atomic dual-layer WAV write: copy → write both layers → rename.
        // Prevents split-state files from partial failures.
        let temp_path = atomic_temp_path(path);
        fs::copy(path, &temp_path)
            .map_err(|e| TagError::Io(format!("Failed to create temp copy: {e}")))?;

        let result = (|| -> Result<(), TagError> {
            for target in &wav_targets {
                let tag_type = TagType::from(target);
                write_tag_layer(
                    &temp_path,
                    tag_type,
                    patch,
                    target.capability(),
                    entry.comment_mode,
                    &mut fields_written,
                    &mut fields_deleted,
                )?;
            }
            Ok(())
        })();

        if let Err(e) = result {
            let _ = fs::remove_file(&temp_path);
            return Err(e);
        }

        fs::rename(&temp_path, path).map_err(|e| {
            let _ = fs::remove_file(&temp_path);
            TagError::Io(format!("Failed to atomically replace file: {e}"))
        })?;
    } else if is_wav {
        // Single-target WAV — direct write, no atomicity concern
        let target = &wav_targets[0];
        let tag_type = TagType::from(target);
        write_tag_layer(
            path,
            tag_type,
            patch,
            target.capability(),
            entry.comment_mode,
            &mut fields_written,
            &mut fields_deleted,
        )?;
    } else {
        // Single tag layer — use primary tag type
        let tag_type = file_type.primary_tag_type();
        write_tag_layer(
            path,
            tag_type,
            patch,
            LayerCapability::Primary,
            entry.comment_mode,
            &mut fields_written,
            &mut fields_deleted,
        )?;
    }

    // De-duplicate (WAV writes to both layers → duplicate entries)
    fields_written.sort();
    fields_written.dedup();
    fields_deleted.sort();
    fields_deleted.dedup();

    Ok(FileWriteResult::Ok {
        path: path_str,
        status: "ok".to_string(),
        fields_written,
        fields_deleted,
        wav_targets: if is_wav {
            Some(
                wav_targets
                    .iter()
                    .map(|t| match t {
                        WavTarget::Id3v2 => "id3v2".to_string(),
                        WavTarget::RiffInfo => "riff_info".to_string(),
                    })
                    .collect(),
            )
        } else {
            None
        },
    })
}

/// Plan and apply one validated patch to a single tag layer.
///
/// The pure planner owns capability filtering, comment merging, deletion, and
/// no-op policy. This function owns only Lofty I/O and plan application.
fn write_tag_layer(
    path: &Path,
    tag_type: TagType,
    patch: &ValidatedTagPatch,
    capability: LayerCapability,
    comment_mode: CommentMode,
    fields_written: &mut Vec<String>,
    fields_deleted: &mut Vec<String>,
) -> Result<(), TagError> {
    #[cfg(test)]
    if TEST_FAIL_TAG_LAYER_WRITE.with(|failure| {
        if failure.get() == Some(tag_type) {
            failure.set(None);
            true
        } else {
            false
        }
    }) {
        return Err(TagError::Io("injected tag-layer write failure".to_string()));
    }

    // Re-read the file for this tag layer (lofty requires read-modify-write
    // per tag type since save_to_path reopens the file).
    // Must read cover art (`true`) so existing pictures survive the round-trip.
    let mut tagged_file = Probe::open(path)
        .map_err(|e| TagError::Io(format!("Failed to open: {e}")))?
        .options(parse_options(true))
        .read()
        .map_err(|e| TagError::Io(format!("Failed to read: {e}")))?;

    let tag = match tagged_file.tag_mut(tag_type) {
        Some(t) => t,
        None => {
            tagged_file.insert_tag(Tag::new(tag_type));
            tagged_file.tag_mut(tag_type).ok_or_else(|| {
                TagError::Unsupported(format!("File does not support {tag_type:?} tags"))
            })?
        }
    };

    let existing = ExistingLayerValues::read(Some(tag), patch);
    let plan = plan_layer_mutation(patch, capability, &existing, comment_mode);
    apply_layer_mutation_plan(tag, tag_type, &plan, fields_written, fields_deleted);

    if !plan.operations.is_empty() {
        tag.save_to_path(path, WriteOptions::default())
            .map_err(|e| TagError::Io(format!("Failed to write {tag_type:?} tag: {e}")))?;
    }

    Ok(())
}

pub(super) fn apply_layer_mutation_plan(
    tag: &mut Tag,
    tag_type: TagType,
    plan: &LayerMutationPlan,
    fields_written: &mut Vec<String>,
    fields_deleted: &mut Vec<String>,
) {
    for operation in &plan.operations {
        let primary_key = operation.field.primary_item_key();
        match &operation.edit {
            TagEdit::Delete => {
                tag.remove_key(primary_key);
                if let Some(secondary_key) = operation.field.secondary_item_key() {
                    tag.remove_key(secondary_key);
                }
                fields_deleted.push(operation.field.as_str().to_string());
            }
            TagEdit::Set(value) => {
                tag.insert_text(primary_key, value.clone());
                // Vorbis Comments use DATE (not YEAR) per spec, and BPM is
                // already the correct key. A secondary write would duplicate
                // either field.
                if tag_type != TagType::VorbisComments
                    && let Some(secondary_key) = operation.field.secondary_item_key()
                {
                    tag.insert_text(secondary_key, value.clone());
                }
                fields_written.push(operation.field.as_str().to_string());
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 3. write_file_tags_dry_run
// ---------------------------------------------------------------------------

/// Preview what a write would do — returns old→new diff for each field.
pub fn write_file_tags_dry_run(entry: &WriteEntry) -> FileDryRunResult {
    let path_str = entry.path.display().to_string();

    let patch = match ValidatedTagPatch::try_from(&entry.tags) {
        Ok(patch) => patch,
        Err(error) => {
            return FileDryRunResult::Error {
                path: path_str,
                status: "error".to_string(),
                error: error.to_string(),
            };
        }
    };

    match write_file_tags_dry_run_inner(entry, &patch) {
        Ok(result) => result,
        Err(e) => FileDryRunResult::Error {
            path: path_str,
            status: "error".to_string(),
            error: e.to_string(),
        },
    }
}

pub(super) fn dry_run_layer_diff(
    patch: &ValidatedTagPatch,
    tag: Option<&Tag>,
    capability: LayerCapability,
    comment_mode: CommentMode,
) -> BTreeMap<String, DryRunChange> {
    let existing = ExistingLayerValues::read(tag, patch);
    plan_layer_mutation(patch, capability, &existing, comment_mode).changes
}

fn write_file_tags_dry_run_inner(
    entry: &WriteEntry,
    patch: &ValidatedTagPatch,
) -> Result<FileDryRunResult, TagError> {
    let path = &entry.path;
    let path_str = path.display().to_string();

    let tagged_file = Probe::open(path)
        .map_err(|e| TagError::Io(format!("Failed to open: {e}")))?
        .options(parse_options(false))
        .read()
        .map_err(|e| TagError::Io(format!("Failed to read: {e}")))?;

    let file_type = tagged_file.file_type();
    let is_wav = file_type == FileType::Wav;

    let wav_targets = if is_wav {
        if entry.wav_targets.is_empty() {
            vec![WavTarget::Id3v2, WavTarget::RiffInfo]
        } else {
            entry.wav_targets.clone()
        }
    } else {
        vec![]
    };

    let (changes, changes_by_layer) = if is_wav {
        let mut changes_by_layer = BTreeMap::new();
        for target in &wav_targets {
            let (key, tag_type, capability) = match target {
                WavTarget::Id3v2 => ("id3v2", TagType::Id3v2, LayerCapability::Id3v2),
                WavTarget::RiffInfo => ("riff_info", TagType::RiffInfo, LayerCapability::RiffInfo),
            };
            changes_by_layer.insert(
                key.to_string(),
                dry_run_layer_diff(
                    patch,
                    tagged_file.tag(tag_type),
                    capability,
                    entry.comment_mode,
                ),
            );
        }

        // Preserve the legacy map's source layer exactly: a single RIFF target
        // mirrors RIFF INFO, while ID3-only, both, and duplicate-target inputs
        // retain the historical ID3v2 comparison.
        let riff_only = wav_targets.len() == 1 && wav_targets[0] == WavTarget::RiffInfo;
        let compatibility_key = if riff_only { "riff_info" } else { "id3v2" };
        let compatibility_diff = changes_by_layer
            .get(compatibility_key)
            .cloned()
            .unwrap_or_else(|| {
                dry_run_layer_diff(
                    patch,
                    tagged_file.tag(TagType::Id3v2),
                    LayerCapability::Id3v2,
                    entry.comment_mode,
                )
            });
        (
            compatibility_diff.into_iter().collect(),
            Some(changes_by_layer),
        )
    } else {
        let primary_tag = tagged_file
            .primary_tag()
            .or_else(|| tagged_file.first_tag());
        (
            dry_run_layer_diff(
                patch,
                primary_tag,
                LayerCapability::Primary,
                entry.comment_mode,
            )
            .into_iter()
            .collect(),
            None,
        )
    };

    Ok(FileDryRunResult::Preview {
        path: path_str,
        status: "preview".to_string(),
        changes,
        changes_by_layer,
        wav_targets: if is_wav {
            Some(
                wav_targets
                    .iter()
                    .map(|t| match t {
                        WavTarget::Id3v2 => "id3v2".to_string(),
                        WavTarget::RiffInfo => "riff_info".to_string(),
                    })
                    .collect(),
            )
        } else {
            None
        },
    })
}
