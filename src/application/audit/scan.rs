//! Audit filesystem scan, freshness, and persistence orchestration.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use rusqlite::Connection;
use serde::Serialize;

use super::resolve::*;
use crate::adapters::audio::tags::{self, FileReadResult};
use crate::adapters::state;
use crate::domain::audit::checks::{
    DetectedIssue, check_filename as check_snapshot_filename, check_tags as check_snapshot_tags,
};
use crate::domain::audit::filename::*;
use crate::domain::audit::{AuditContext, IssueType, TagSnapshot};

fn tag_snapshot(result: &FileReadResult) -> TagSnapshot {
    match result {
        FileReadResult::Single { tags, .. } => TagSnapshot::Single { tags: tags.clone() },
        FileReadResult::Wav {
            id3v2,
            riff_info,
            tag3_missing,
            ..
        } => TagSnapshot::Wav {
            id3v2: id3v2.clone(),
            riff_info: riff_info.clone(),
            tag3_missing: tag3_missing.clone(),
        },
        FileReadResult::Error { .. } => TagSnapshot::Error,
    }
}

fn check_tags(
    path: &Path,
    result: &FileReadResult,
    context: &AuditContext,
    skip: &HashSet<IssueType>,
) -> Vec<crate::domain::audit::checks::DetectedIssue> {
    check_snapshot_tags(path, &tag_snapshot(result), context, skip)
}

fn check_filename(
    path: &Path,
    result: &FileReadResult,
    context: &AuditContext,
    skip: &HashSet<IssueType>,
) -> Vec<crate::domain::audit::checks::DetectedIssue> {
    check_snapshot_filename(path, &tag_snapshot(result), context, skip)
}

// ---------------------------------------------------------------------------
// Scan operation
// ---------------------------------------------------------------------------

use crate::audio::AUDIO_EXTENSIONS;
const BATCH_SIZE: usize = 500;

#[derive(Debug, Serialize)]
pub struct ScanSummary {
    pub files_in_scope: usize,
    pub scanned: usize,
    pub failed_reads: usize,
    pub skipped_unchanged: usize,
    pub missing_from_disk: usize,
    pub skipped_issue_types: Vec<String>,
    pub new_issues: HashMap<String, usize>,
    pub auto_resolved: HashMap<String, usize>,
    pub total_open: i64,
    pub total_resolved: i64,
    pub total_accepted: i64,
    pub total_deferred: i64,
    pub warnings: Vec<String>,
}

pub(crate) fn enforce_trailing_slash(scope: &str) -> String {
    if scope.ends_with('/') {
        scope.to_string()
    } else {
        format!("{scope}/")
    }
}

struct WalkResult {
    files: Vec<std::path::PathBuf>,
    warnings: Vec<String>,
    had_errors: bool,
}

fn walk_audio_files(scope: &Path) -> Result<WalkResult, String> {
    if !scope.is_dir() {
        return Err(format!("Not a directory: {}", scope.display()));
    }

    let mut files = Vec::new();
    let mut warnings = Vec::new();
    let mut had_errors = false;
    let mut dirs = vec![scope.to_path_buf()];

    while let Some(dir) = dirs.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) => {
                warnings.push(format!("Cannot read {}: {e}", dir.display()));
                had_errors = true;
                continue;
            }
        };

        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    warnings.push(format!("Dir entry error in {}: {e}", dir.display()));
                    had_errors = true;
                    continue;
                }
            };

            let file_type = match entry.file_type() {
                Ok(ft) => ft,
                Err(e) => {
                    warnings.push(format!("Cannot read entry type in {}: {e}", dir.display()));
                    had_errors = true;
                    continue;
                }
            };

            if file_type.is_symlink() {
                continue;
            }

            let path = entry.path();

            if file_type.is_dir() {
                dirs.push(path);
                continue;
            }

            if !file_type.is_file() {
                continue;
            }

            let is_audio = path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| AUDIO_EXTENSIONS.contains(&e.to_lowercase().as_str()));
            if is_audio {
                files.push(path);
            }
        }
    }

    files.sort();
    Ok(WalkResult {
        files,
        warnings,
        had_errors,
    })
}

const AUDIT_FRESHNESS_VERSION: &str = "v2";

fn audit_context_freshness_token(context: AuditContext) -> &'static str {
    match context {
        AuditContext::AlbumTrack => "album",
        AuditContext::LooseTrack => "loose",
    }
}

pub(super) fn audit_freshness_key(
    modified: Option<std::time::SystemTime>,
    context: AuditContext,
) -> Option<String> {
    let modified_nanos = modified?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos();
    Some(format!(
        "{AUDIT_FRESHNESS_VERSION}:{modified_nanos}:{}",
        audit_context_freshness_token(context)
    ))
}

fn audit_freshness_key_from_metadata(
    metadata: &std::fs::Metadata,
    context: AuditContext,
) -> Option<String> {
    audit_freshness_key(metadata.modified().ok(), context)
}

pub(super) fn is_successful_audit_freshness_key(value: &str) -> bool {
    let mut parts = value.split(':');
    matches!(parts.next(), Some(AUDIT_FRESHNESS_VERSION))
        && parts
            .next()
            .is_some_and(|nanos| nanos.parse::<u128>().is_ok())
        && matches!(parts.next(), Some("album" | "loose"))
        && parts.next().is_none()
}

#[derive(Clone, Copy)]
enum AuditRetryKind {
    Read,
    Metadata,
}

fn retry_audit_freshness_key(kind: AuditRetryKind, attempt_nanos: u128) -> String {
    let kind = match kind {
        AuditRetryKind::Read => "read",
        AuditRetryKind::Metadata => "metadata",
    };
    format!("retry:{kind}:{attempt_nanos}")
}

fn audit_attempt_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

pub(crate) fn now_iso() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

fn delete_missing_files_if_walk_complete(
    conn: &Connection,
    scope: &str,
    disk_path_set: &HashSet<String>,
    walk_had_errors: bool,
    warnings: &mut Vec<String>,
) -> Result<usize, String> {
    if walk_had_errors {
        warnings.push(
            "Skipped missing-file cleanup because filesystem walk had read errors; existing audit rows were preserved."
                .to_string(),
        );
        Ok(0)
    } else {
        state::delete_missing_audit_files(conn, scope, disk_path_set)
            .map_err(|e| format!("DB error deleting missing files: {e}"))
    }
}

pub fn scan(
    conn: &Connection,
    scope: &str,
    revalidate: bool,
    skip_issue_types: &HashSet<IssueType>,
    rekordbox_imported: Option<&HashSet<String>>,
) -> Result<ScanSummary, String> {
    scan_with_freshness_key_provider(
        conn,
        scope,
        revalidate,
        skip_issue_types,
        rekordbox_imported,
        audit_freshness_key_from_metadata,
    )
}

fn scan_with_freshness_key_provider(
    conn: &Connection,
    scope: &str,
    revalidate: bool,
    skip_issue_types: &HashSet<IssueType>,
    rekordbox_imported: Option<&HashSet<String>>,
    freshness_key_for: fn(&std::fs::Metadata, AuditContext) -> Option<String>,
) -> Result<ScanSummary, String> {
    let scope = enforce_trailing_slash(scope);
    if scope == "/" {
        return Err("Scope must not be empty or root (/)".to_string());
    }
    let scope_path = Path::new(&scope);

    // 1. Walk filesystem
    let walk_result = walk_audio_files(scope_path)?;
    let WalkResult {
        files: disk_files,
        mut warnings,
        had_errors: walk_had_errors,
    } = walk_result;
    let files_in_scope = disk_files.len();

    // 2. Load existing audit_files for this scope
    let existing = state::get_audit_files_in_scope(conn, &scope)
        .map_err(|e| format!("DB error loading audit files: {e}"))?;
    let existing_map: HashMap<String, state::AuditFile> =
        existing.into_iter().map(|f| (f.path.clone(), f)).collect();

    let disk_path_set: HashSet<String> =
        disk_files.iter().map(|p| p.display().to_string()).collect();

    // 3. Delete missing files
    let missing_from_disk = delete_missing_files_if_walk_complete(
        conn,
        &scope,
        &disk_path_set,
        walk_had_errors,
        &mut warnings,
    )?;

    let mut scanned = 0usize;
    let mut failed_reads = 0usize;
    let mut skipped_unchanged = 0usize;
    let mut new_issues: HashMap<String, usize> = HashMap::new();
    let mut auto_resolved: HashMap<String, usize> = HashMap::new();

    // Pre-compute directory-level imported set for TechSpecsInDir annotation
    let imported_dirs: HashSet<String> = rekordbox_imported
        .map(|paths| {
            paths
                .iter()
                .filter_map(|p| {
                    std::path::Path::new(p)
                        .parent()
                        .map(|d| d.to_string_lossy().into_owned())
                })
                .collect()
        })
        .unwrap_or_default();

    // Pre-pass: detect album dirs by counting track-number prefixes
    let album_dirs = detect_album_dirs(&disk_files);

    // 4. Process files in batches (transaction auto-rolls-back on early exit)
    let mut batch_count = 0usize;
    let now = now_iso();

    let mut tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("DB error starting transaction: {e}"))?;

    for file_path in &disk_files {
        let path_str = file_path.display().to_string();
        let metadata = match std::fs::metadata(file_path) {
            Ok(m) => m,
            Err(e) => {
                warnings.push(format!("Cannot stat {path_str}: {e}"));
                continue;
            }
        };
        let size = metadata.len() as i64;
        let context = classify_track_context(file_path, &album_dirs);
        let expected_freshness_key = freshness_key_for(&metadata, context);
        let existing_file = existing_map.get(&path_str);
        let is_unchanged = !revalidate
            && existing_file.is_some_and(|file| {
                file.file_size == size
                    && expected_freshness_key.as_deref().is_some_and(|expected| {
                        is_successful_audit_freshness_key(expected)
                            && file.freshness_key == expected
                    })
            });

        if is_unchanged {
            skipped_unchanged += 1;
        } else {
            let read_result = tags::read_file_tags(file_path, None, false);

            if let FileReadResult::Error { error, .. } = &read_result {
                failed_reads += 1;
                warnings.push(format!(
                    "Tag read failed for {path_str}: {error}; file will be retried."
                ));
                let retry_key =
                    retry_audit_freshness_key(AuditRetryKind::Read, audit_attempt_nanos());
                state::upsert_audit_file(&tx, &path_str, &now, &retry_key, size)
                    .map_err(|e| format!("DB error upserting failed-read file: {e}"))?;
            } else {
                let mut detected: Vec<DetectedIssue> = Vec::new();
                detected.extend(check_tags(
                    file_path,
                    &read_result,
                    &context,
                    skip_issue_types,
                ));
                detected.extend(check_filename(
                    file_path,
                    &read_result,
                    &context,
                    skip_issue_types,
                ));

                // Annotate rename-type issues with Rekordbox import status
                if let Some(imported_set) = rekordbox_imported {
                    for issue in &mut detected {
                        let is_imported = match issue.issue_type {
                            IssueType::OriginalMixSuffix => Some(imported_set.contains(&path_str)),
                            IssueType::TechSpecsInDir => {
                                let parent_dir = file_path
                                    .parent()
                                    .map(|d| d.to_string_lossy().into_owned())
                                    .unwrap_or_default();
                                Some(imported_dirs.contains(&parent_dir))
                            }
                            _ => None,
                        };
                        if let (Some(imported), Some(detail)) = (is_imported, &issue.detail)
                            && let Ok(mut obj) = serde_json::from_str::<serde_json::Value>(detail)
                        {
                            obj["imported"] = serde_json::Value::Bool(imported);
                            issue.detail = Some(obj.to_string());
                        }
                    }
                }

                let persisted_freshness_key = expected_freshness_key.unwrap_or_else(|| {
                    warnings.push(format!(
                        "Audited {path_str}, but its modified time was unavailable; file will be retried."
                    ));
                    retry_audit_freshness_key(
                        AuditRetryKind::Metadata,
                        audit_attempt_nanos(),
                    )
                });
                state::upsert_audit_file(&tx, &path_str, &now, &persisted_freshness_key, size)
                    .map_err(|e| format!("DB error upserting file: {e}"))?;

                let detected_types: Vec<&str> =
                    detected.iter().map(|d| d.issue_type.as_str()).collect();
                for issue in &detected {
                    state::upsert_audit_issue(
                        &tx,
                        &path_str,
                        issue.issue_type.as_str(),
                        issue.detail.as_deref(),
                        "open",
                        &now,
                    )
                    .map_err(|e| format!("DB error upserting issue: {e}"))?;

                    *new_issues.entry(issue.issue_type.to_string()).or_insert(0) += 1;
                }

                // Auto-resolve issues no longer detected (for changed/re-read files).
                if existing_file.is_some() {
                    // Skipped issue types should not be auto-resolved — we didn't check them
                    let mut types_still_open: Vec<&str> = detected_types.clone();
                    for skip_type in skip_issue_types {
                        let s = skip_type.as_str();
                        if !types_still_open.contains(&s) {
                            types_still_open.push(s);
                        }
                    }

                    let resolved_count = state::mark_issues_resolved_for_path(
                        &tx,
                        &path_str,
                        &types_still_open,
                        &now,
                    )
                    .map_err(|e| format!("DB error resolving issues: {e}"))?;
                    if resolved_count > 0 {
                        *auto_resolved.entry("_total".to_string()).or_insert(0) += resolved_count;
                    }
                }

                scanned += 1;
            }
        }

        batch_count += 1;
        if batch_count >= BATCH_SIZE {
            tx.commit()
                .map_err(|e| format!("DB error committing batch: {e}"))?;
            tx = conn
                .unchecked_transaction()
                .map_err(|e| format!("DB error starting transaction: {e}"))?;
            batch_count = 0;
        }
    }

    tx.commit()
        .map_err(|e| format!("DB error committing final batch: {e}"))?;

    // 4b. Refresh import annotations on ALL open rename-type issues in scope.
    // This catches issues that were skipped (unchanged files) but whose Rekordbox
    // import status may have changed since the last scan.
    if let Some(imported_set) = rekordbox_imported {
        let rename_types = [
            IssueType::OriginalMixSuffix.as_str(),
            IssueType::TechSpecsInDir.as_str(),
        ];
        let issues = state::get_open_issues_by_types(conn, &scope, &rename_types)
            .map_err(|e| format!("DB error querying issues for import refresh: {e}"))?;
        for (id, path, issue_type, detail) in &issues {
            let is_imported = match issue_type.as_str() {
                t if t == IssueType::OriginalMixSuffix.as_str() => imported_set.contains(path),
                t if t == IssueType::TechSpecsInDir.as_str() => {
                    let parent_dir = std::path::Path::new(path)
                        .parent()
                        .map(|d| d.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    imported_dirs.contains(&parent_dir)
                }
                _ => continue,
            };
            if let Some(detail_str) = detail
                && let Ok(mut obj) = serde_json::from_str::<serde_json::Value>(detail_str)
            {
                obj["imported"] = serde_json::Value::Bool(is_imported);
                let updated = obj.to_string();
                if updated != *detail_str {
                    state::update_audit_issue_detail(conn, *id, &updated)
                        .map_err(|e| format!("DB error updating import annotation: {e}"))?;
                }
            }
        }
    }

    // 5. Build summary from DB
    let summary = state::get_audit_summary(conn, &scope)
        .map_err(|e| format!("DB error getting summary: {e}"))?;

    let counts = aggregate_status_counts(&summary);

    let skipped_names: Vec<String> = skip_issue_types
        .iter()
        .map(std::string::ToString::to_string)
        .collect();

    Ok(ScanSummary {
        files_in_scope,
        scanned,
        failed_reads,
        skipped_unchanged,
        missing_from_disk,
        skipped_issue_types: skipped_names,
        new_issues,
        auto_resolved,
        total_open: counts.open,
        total_resolved: counts.resolved,
        total_accepted: counts.accepted,
        total_deferred: counts.deferred,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_minimal_pcm_wav(path: &Path) {
        let data_size: u32 = 2;
        let file_size = 36 + data_size;
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&file_size.to_le_bytes());
        wav.extend_from_slice(b"WAVE");
        wav.extend_from_slice(b"fmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&44100u32.to_le_bytes());
        wav.extend_from_slice(&88200u32.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_size.to_le_bytes());
        wav.extend_from_slice(&[0u8; 2]);
        std::fs::write(path, wav).unwrap();
    }

    fn open_scan_test_store(dir: &tempfile::TempDir) -> Connection {
        let db_path = dir.path().join("internal.sqlite3");
        state::open(db_path.to_str().unwrap()).unwrap()
    }

    // -- classify_track_context --

    #[test]
    fn scan_rejects_empty_scope() {
        let result = enforce_trailing_slash("");
        assert_eq!(result, "/");
    }

    #[test]
    fn audit_read_failure_is_retried() {
        let dir = tempfile::tempdir().unwrap();
        let conn = open_scan_test_store(&dir);
        let unreadable = dir.path().join("broken.flac");
        std::fs::write(&unreadable, b"not a valid FLAC file").unwrap();

        let first = scan(
            &conn,
            dir.path().to_str().unwrap(),
            false,
            &HashSet::new(),
            None,
        )
        .unwrap();
        let second = scan(
            &conn,
            dir.path().to_str().unwrap(),
            false,
            &HashSet::new(),
            None,
        )
        .unwrap();

        assert_eq!(
            first.scanned, 0,
            "a failed tag read is not a completed scan"
        );
        assert_eq!(first.failed_reads, 1);
        assert_eq!(second.scanned, 0, "the retry should fail, not scan cleanly");
        assert_eq!(second.failed_reads, 1);
        assert_eq!(
            second.skipped_unchanged, 0,
            "a failed tag read must remain retryable",
        );
        assert!(
            second
                .warnings
                .iter()
                .any(|warning| warning.contains("Tag read failed")
                    && warning.contains("broken.flac"))
        );
        let persisted = state::get_audit_file(&conn, unreadable.to_str().unwrap())
            .unwrap()
            .unwrap();
        assert!(persisted.freshness_key.starts_with("retry:read:"));
    }

    #[test]
    fn audit_read_failure_preserves_existing_issues() {
        let dir = tempfile::tempdir().unwrap();
        let conn = open_scan_test_store(&dir);
        let unreadable = dir.path().join("broken.flac");
        std::fs::write(&unreadable, b"not a valid FLAC file").unwrap();
        let path = unreadable.to_str().unwrap();
        let size = std::fs::metadata(&unreadable).unwrap().len() as i64;
        state::upsert_audit_file(&conn, path, "before", "legacy-freshness", size).unwrap();
        state::upsert_audit_issue(
            &conn,
            path,
            IssueType::EmptyArtist.as_str(),
            Some("existing detail"),
            "open",
            "before",
        )
        .unwrap();

        let summary = scan(
            &conn,
            dir.path().to_str().unwrap(),
            true,
            &HashSet::new(),
            None,
        )
        .unwrap();
        let issues = state::get_audit_issues(
            &conn,
            &enforce_trailing_slash(dir.path().to_str().unwrap()),
            None,
            None,
            100,
            0,
        )
        .unwrap();

        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].status, "open");
        assert_eq!(issues[0].detail.as_deref(), Some("existing detail"));
        assert_eq!(summary.scanned, 0, "a failed revalidation is not scanned");
        assert_eq!(summary.failed_reads, 1);
        assert!(summary.auto_resolved.is_empty());
        let persisted = state::get_audit_file(&conn, path).unwrap().unwrap();
        assert!(persisted.freshness_key.starts_with("retry:read:"));
    }

    #[test]
    fn audit_freshness_key_distinguishes_subsecond_mtimes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mtime.wav");
        write_minimal_pcm_wav(&path);
        let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        let same_second = 1_700_000_000;
        let first_time = std::time::UNIX_EPOCH + std::time::Duration::new(same_second, 123_000_000);
        file.set_times(std::fs::FileTimes::new().set_modified(first_time))
            .unwrap();
        let first_metadata = std::fs::metadata(&path).unwrap();

        let second_time =
            std::time::UNIX_EPOCH + std::time::Duration::new(same_second, 987_000_000);
        file.set_times(std::fs::FileTimes::new().set_modified(second_time))
            .unwrap();
        let second_metadata = std::fs::metadata(&path).unwrap();

        assert_ne!(
            first_metadata.modified().unwrap(),
            second_metadata.modified().unwrap(),
            "fixture must retain subsecond mtime precision",
        );
        let first_key =
            audit_freshness_key(first_metadata.modified().ok(), AuditContext::LooseTrack).unwrap();
        let second_key =
            audit_freshness_key(second_metadata.modified().ok(), AuditContext::LooseTrack).unwrap();
        assert_ne!(first_key, second_key);
    }

    #[test]
    fn audit_freshness_key_context_and_domains() {
        let modified = std::time::UNIX_EPOCH + std::time::Duration::new(42, 123);
        let album = audit_freshness_key(Some(modified), AuditContext::AlbumTrack).unwrap();
        let loose = audit_freshness_key(Some(modified), AuditContext::LooseTrack).unwrap();
        assert_eq!(album, "v2:42000000123:album");
        assert_eq!(loose, "v2:42000000123:loose");
        assert_ne!(album, loose);
        assert!(is_successful_audit_freshness_key(&album));
        assert!(is_successful_audit_freshness_key(&loose));

        assert!(audit_freshness_key(None, AuditContext::LooseTrack).is_none());
        assert!(
            audit_freshness_key(
                Some(std::time::UNIX_EPOCH - std::time::Duration::from_nanos(1)),
                AuditContext::LooseTrack,
            )
            .is_none()
        );

        let read_retry = retry_audit_freshness_key(AuditRetryKind::Read, 99);
        let metadata_retry = retry_audit_freshness_key(AuditRetryKind::Metadata, 100);
        assert_eq!(read_retry, "retry:read:99");
        assert_eq!(metadata_retry, "retry:metadata:100");
        for not_successful in [
            read_retry.as_str(),
            metadata_retry.as_str(),
            "2026-02-20T10:00:00Z",
            "v2:not-nanos:loose",
            "v2:123:other",
        ] {
            assert!(!is_successful_audit_freshness_key(not_successful));
        }
    }

    #[test]
    fn album_context_change_reaudits_unchanged_file() {
        let dir = tempfile::tempdir().unwrap();
        let conn = open_scan_test_store(&dir);
        let first_path = dir.path().join("01 First.wav");
        write_minimal_pcm_wav(&first_path);

        let first = scan(
            &conn,
            dir.path().to_str().unwrap(),
            false,
            &HashSet::new(),
            None,
        )
        .unwrap();
        assert_eq!(first.scanned, 1);

        let second_path = dir.path().join("02 Second.wav");
        write_minimal_pcm_wav(&second_path);
        let after_sibling_added = scan(
            &conn,
            dir.path().to_str().unwrap(),
            false,
            &HashSet::new(),
            None,
        )
        .unwrap();
        assert_eq!(
            after_sibling_added.scanned, 2,
            "the unchanged first file must be reaudited after its context changes",
        );
        assert_eq!(after_sibling_added.skipped_unchanged, 0);

        let stable_album = scan(
            &conn,
            dir.path().to_str().unwrap(),
            false,
            &HashSet::new(),
            None,
        )
        .unwrap();
        assert_eq!(stable_album.scanned, 0);
        assert_eq!(stable_album.skipped_unchanged, 2);
    }

    #[test]
    fn audit_metadata_freshness_failure_is_scanned_and_retried() {
        let dir = tempfile::tempdir().unwrap();
        let conn = open_scan_test_store(&dir);
        let path = dir.path().join("Artist - Track.wav");
        write_minimal_pcm_wav(&path);
        let path_str = path.to_str().unwrap();
        let size = std::fs::metadata(&path).unwrap().len() as i64;
        state::upsert_audit_file(&conn, path_str, "before", "legacy-freshness", size).unwrap();
        state::upsert_audit_issue(
            &conn,
            path_str,
            IssueType::GenreSet.as_str(),
            None,
            "open",
            "before",
        )
        .unwrap();

        let missing_metadata_key = scan_with_freshness_key_provider(
            &conn,
            dir.path().to_str().unwrap(),
            false,
            &HashSet::new(),
            None,
            |_, _| None,
        )
        .unwrap();
        assert_eq!(missing_metadata_key.scanned, 1);
        assert_eq!(missing_metadata_key.failed_reads, 0);
        assert_eq!(missing_metadata_key.skipped_unchanged, 0);
        assert!(missing_metadata_key.warnings.iter().any(|warning| {
            warning.contains("modified time was unavailable") && warning.contains("Track.wav")
        }));
        let retry = state::get_audit_file(&conn, path_str).unwrap().unwrap();
        assert!(retry.freshness_key.starts_with("retry:metadata:"));
        let issues = state::get_audit_issues(
            &conn,
            &enforce_trailing_slash(dir.path().to_str().unwrap()),
            None,
            Some(IssueType::GenreSet.as_str()),
            100,
            0,
        )
        .unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].status, "resolved");

        let recovered = scan(
            &conn,
            dir.path().to_str().unwrap(),
            false,
            &HashSet::new(),
            None,
        )
        .unwrap();
        assert_eq!(recovered.scanned, 1);
        assert_eq!(recovered.failed_reads, 0);
        let successful = state::get_audit_file(&conn, path_str).unwrap().unwrap();
        assert!(successful.freshness_key.starts_with("v2:"));

        let unchanged = scan(
            &conn,
            dir.path().to_str().unwrap(),
            false,
            &HashSet::new(),
            None,
        )
        .unwrap();
        assert_eq!(unchanged.scanned, 0);
        assert_eq!(unchanged.failed_reads, 0);
        assert_eq!(unchanged.skipped_unchanged, 1);
    }

    #[test]
    fn audit_successful_unchanged_file_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let conn = open_scan_test_store(&dir);
        write_minimal_pcm_wav(&dir.path().join("Artist - Track.wav"));

        let first = scan(
            &conn,
            dir.path().to_str().unwrap(),
            false,
            &HashSet::new(),
            None,
        )
        .unwrap();
        let second = scan(
            &conn,
            dir.path().to_str().unwrap(),
            false,
            &HashSet::new(),
            None,
        )
        .unwrap();
        assert_eq!(first.scanned, 1);
        assert_eq!(first.failed_reads, 0);
        assert_eq!(second.scanned, 0);
        assert_eq!(second.failed_reads, 0);
        assert_eq!(second.skipped_unchanged, 1);
    }

    #[test]
    fn audit_legacy_freshness_rescans_once() {
        let dir = tempfile::tempdir().unwrap();
        let conn = open_scan_test_store(&dir);
        let path = dir.path().join("Artist - Track.wav");
        write_minimal_pcm_wav(&path);
        let path_str = path.to_str().unwrap();
        let size = std::fs::metadata(&path).unwrap().len() as i64;
        state::upsert_audit_file(&conn, path_str, "before", "2026-02-20T10:00:00Z", size).unwrap();

        let refreshed = scan(
            &conn,
            dir.path().to_str().unwrap(),
            false,
            &HashSet::new(),
            None,
        )
        .unwrap();
        assert_eq!(refreshed.scanned, 1);
        let persisted = state::get_audit_file(&conn, path_str).unwrap().unwrap();
        assert!(persisted.freshness_key.starts_with("v2:"));

        let stable = scan(
            &conn,
            dir.path().to_str().unwrap(),
            false,
            &HashSet::new(),
            None,
        )
        .unwrap();
        assert_eq!(stable.scanned, 0);
        assert_eq!(stable.skipped_unchanged, 1);
    }

    #[test]
    fn unrelated_album_context_change_does_not_reaudit_other_directory() {
        let dir = tempfile::tempdir().unwrap();
        let conn = open_scan_test_store(&dir);
        let stable_dir = dir.path().join("stable");
        let changing_dir = dir.path().join("changing");
        std::fs::create_dir(&stable_dir).unwrap();
        std::fs::create_dir(&changing_dir).unwrap();
        write_minimal_pcm_wav(&stable_dir.join("Artist - Stable.wav"));
        write_minimal_pcm_wav(&changing_dir.join("01 First.wav"));
        let first = scan(
            &conn,
            dir.path().to_str().unwrap(),
            false,
            &HashSet::new(),
            None,
        )
        .unwrap();
        assert_eq!(first.scanned, 2);

        write_minimal_pcm_wav(&changing_dir.join("02 Second.wav"));
        let changed = scan(
            &conn,
            dir.path().to_str().unwrap(),
            false,
            &HashSet::new(),
            None,
        )
        .unwrap();
        assert_eq!(changed.scanned, 2);
        assert_eq!(changed.skipped_unchanged, 1);
    }

    #[test]
    fn audit_failed_reads_respect_transaction_batching() {
        let dir = tempfile::tempdir().unwrap();
        let conn = open_scan_test_store(&dir);
        for index in 0..=BATCH_SIZE {
            let path = dir.path().join(format!("{index:04}-broken.flac"));
            std::fs::write(path, b"not a valid FLAC file").unwrap();
        }
        conn.execute_batch(
            "CREATE TRIGGER fail_after_first_audit_batch
             BEFORE INSERT ON audit_files
             WHEN NEW.path LIKE '%/0500-broken.flac'
             BEGIN
                 SELECT RAISE(ABORT, 'stop after committed batch');
             END;",
        )
        .unwrap();

        let error = scan(
            &conn,
            dir.path().to_str().unwrap(),
            false,
            &HashSet::new(),
            None,
        )
        .unwrap_err();
        assert!(error.contains("upserting failed-read file"));
        let committed: i64 = conn
            .query_row("SELECT COUNT(*) FROM audit_files", [], |row| row.get(0))
            .unwrap();
        assert_eq!(committed, BATCH_SIZE as i64);
    }

    // Finding 9: NN - Title parsing

    #[test]
    fn skip_missing_cleanup_when_walk_has_errors() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("internal.sqlite3");
        let conn = state::open(db_path.to_str().unwrap()).unwrap();

        state::upsert_audit_file(&conn, "/music/a.flac", "t1", "m1", 100).unwrap();

        let disk_path_set: HashSet<String> = HashSet::new();
        let mut warnings = Vec::new();
        let removed = delete_missing_files_if_walk_complete(
            &conn,
            "/music/",
            &disk_path_set,
            true,
            &mut warnings,
        )
        .unwrap();

        assert_eq!(removed, 0);
        let files = state::get_audit_files_in_scope(&conn, "/music/").unwrap();
        assert_eq!(files.len(), 1);
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("Skipped missing-file cleanup"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn scan_skips_missing_cleanup_when_walk_hits_unreadable_subdir() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("internal.sqlite3");
        let conn = state::open(db_path.to_str().unwrap()).unwrap();

        let ok_file = dir.path().join("ok.flac");
        std::fs::write(&ok_file, b"not-audio").unwrap();

        let blocked_dir = dir.path().join("blocked");
        std::fs::create_dir(&blocked_dir).unwrap();
        let blocked_file = blocked_dir.join("hidden.flac");
        std::fs::write(&blocked_file, b"not-audio").unwrap();

        let ok_path = ok_file.to_str().unwrap();
        let blocked_path = blocked_file.to_str().unwrap();
        state::upsert_audit_file(&conn, ok_path, "t1", "m1", 1).unwrap();
        state::upsert_audit_file(&conn, blocked_path, "t1", "m1", 1).unwrap();

        let original_perms = std::fs::metadata(&blocked_dir).unwrap().permissions();
        let mut no_access = original_perms.clone();
        no_access.set_mode(0o000);
        std::fs::set_permissions(&blocked_dir, no_access).unwrap();

        let scan_result = scan(
            &conn,
            dir.path().to_str().unwrap(),
            false,
            &HashSet::new(),
            None,
        );

        std::fs::set_permissions(&blocked_dir, original_perms).unwrap();

        let summary = scan_result.expect("scan should continue with warnings");
        assert_eq!(summary.missing_from_disk, 0);
        assert!(summary.warnings.iter().any(|w| w.contains("Cannot read")));
        assert!(
            summary
                .warnings
                .iter()
                .any(|w| w.contains("Skipped missing-file cleanup"))
        );
        assert!(
            state::get_audit_file(&conn, blocked_path)
                .unwrap()
                .is_some()
        );
    }

    // -- has_year_suffix: compound parenthetical content --

    #[test]
    fn boundary_audit_scan_is_read_only() {
        let directory = tempfile::tempdir().unwrap();
        let audio_path = directory.path().join("Artist - Track.wav");
        write_minimal_pcm_wav(&audio_path);
        let audio_before = std::fs::read(&audio_path).unwrap();
        let store = open_scan_test_store(&directory);
        let summary = scan(
            &store,
            directory.path().to_str().unwrap(),
            false,
            &HashSet::new(),
            None,
        )
        .unwrap();
        assert_eq!(summary.files_in_scope, 1);
        assert!(
            state::get_audit_file(&store, audio_path.to_str().unwrap())
                .unwrap()
                .is_some()
        );
        assert_eq!(std::fs::read(&audio_path).unwrap(), audio_before);
    }
}
