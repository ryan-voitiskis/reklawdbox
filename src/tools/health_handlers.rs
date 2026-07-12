use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use rmcp::ErrorData as McpError;
use rmcp::model::CallToolResult;

use super::{
    OffsetPage, ReklawdboxServer, db_error, mcp_internal_error, offset_page_bounds, ok_json,
    ok_structured_json, scan_audio_directory,
};
use crate::audio;
use crate::db;
use crate::tools::params::{
    DuplicateDetectionLevel, ScanBrokenLinksParams, ScanDuplicatesParams, ScanOrphanFilesParams,
    ScanPlaylistCoverageParams,
};

pub(super) fn handle_scan_playlist_coverage(
    conn: std::sync::MutexGuard<'_, rusqlite::Connection>,
    params: ScanPlaylistCoverageParams,
) -> Result<CallToolResult, McpError> {
    let search = params
        .filters
        .into_search_params(true, params.limit, params.offset)
        .map_err(mcp_internal_error)?;

    let result = db::tracks_not_in_any_playlist(&conn, &search).map_err(db_error)?;

    let coverage_pct = if result.total_tracks > 0 {
        ((result.total_tracks - result.uncovered_count) as f64 / result.total_tracks as f64) * 100.0
    } else {
        100.0
    };

    let json_result = serde_json::json!({
        "total_tracks": result.total_tracks,
        "uncovered_count": result.uncovered_count,
        "coverage_pct": (coverage_pct * 10.0).round() / 10.0,
        "tracks": result.tracks,
    });

    ok_json(&json_result)
}

pub(super) async fn handle_scan_broken_links(
    server: &ReklawdboxServer,
    params: ScanBrokenLinksParams,
) -> Result<CallToolResult, McpError> {
    let suggest_relocations = params.suggest_relocations.unwrap_or(true);
    let limit = params.limit.unwrap_or(200).min(500) as usize;
    let offset = params.offset.unwrap_or(0) as usize;

    // Phase 1: DB queries (scoped to drop conn before await)
    let (all_paths, roots) = {
        let conn = server.rekordbox_conn()?;
        let all_paths =
            db::all_track_paths(&conn, params.path_prefix.as_deref()).map_err(db_error)?;
        let roots = if suggest_relocations {
            Some(db::content_roots(&conn, true).map_err(db_error)?)
        } else {
            None
        };
        (all_paths, roots)
    };

    // Phase 2: Filesystem checks
    let (checked, broken_links, relocation_map) = tokio::task::spawn_blocking(move || {
        let checked = all_paths.len();
        let mut broken: Vec<db::TrackPathEntry> = Vec::new();
        for entry in &all_paths {
            if audio::resolve_audio_path(&entry.path).is_err() {
                broken.push(entry.clone());
            }
        }

        let remap = if let Some(roots) = roots {
            if !broken.is_empty() {
                let mut map: HashMap<String, Vec<String>> = HashMap::new();
                for root in &roots {
                    if let Ok(files) = scan_audio_directory(root, true, None) {
                        for file_path in files {
                            if let Some(fname) = std::path::Path::new(&file_path)
                                .file_name()
                                .and_then(|n| n.to_str())
                            {
                                map.entry(fname.to_lowercase()).or_default().push(file_path);
                            }
                        }
                    }
                }
                Some(map)
            } else {
                None
            }
        } else {
            None
        };

        (checked, broken, remap)
    })
    .await
    .map_err(|e| mcp_internal_error(format!("Task join error: {e}")))?;

    let total_broken = broken_links.len();
    let paginated: Vec<_> = broken_links.iter().skip(offset).take(limit).collect();

    let broken_results: Vec<serde_json::Value> = paginated
        .iter()
        .map(|entry| {
            let mut obj = serde_json::json!({
                "track_id": entry.id,
                "artist": entry.artist,
                "title": entry.title,
                "db_path": entry.path,
            });
            if let Some(ref map) = relocation_map {
                let filename = std::path::Path::new(entry.path.as_str())
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("");
                let decoded = percent_encoding::percent_decode_str(filename)
                    .decode_utf8()
                    .map(|s| s.to_string())
                    .unwrap_or_default();
                let mut suggestions: Vec<String> = Vec::new();
                if let Some(paths) = map.get(&filename.to_lowercase()) {
                    suggestions.extend(paths.iter().cloned());
                }
                if !decoded.is_empty()
                    && decoded != filename
                    && let Some(paths) = map.get(&decoded.to_lowercase())
                {
                    for p in paths {
                        if !suggestions.contains(p) {
                            suggestions.push(p.clone());
                        }
                    }
                }
                obj["suggested_relocations"] = serde_json::json!(suggestions);
            }
            obj
        })
        .collect();

    let result = serde_json::json!({
        "checked": checked,
        "broken_count": total_broken,
        "broken_links": broken_results,
    });

    ok_json(&result)
}

pub(super) async fn handle_scan_orphan_files(
    server: &ReklawdboxServer,
    params: ScanOrphanFilesParams,
) -> Result<CallToolResult, McpError> {
    let limit = params.limit.unwrap_or(200).min(1000) as usize;

    // Phase 1: DB queries (scoped to drop conn before await)
    let imported_by_root = {
        let conn = server.rekordbox_conn()?;
        let roots = if let Some(ref prefix) = params.path_prefix {
            vec![prefix.clone()]
        } else {
            db::content_roots(&conn, true).map_err(db_error)?
        };

        let mut imported_by_root: Vec<(String, usize, HashSet<String>)> = Vec::new();
        for root in &roots {
            let imported = db::paths_imported_in_scope(&conn, root).map_err(db_error)?;
            let raw_count = imported.len();
            let mut expanded = HashSet::new();
            for path in &imported {
                expanded.insert(path.clone());
                if let Ok(decoded) = percent_encoding::percent_decode_str(path).decode_utf8() {
                    let decoded = decoded.to_string();
                    if decoded != *path {
                        expanded.insert(decoded);
                    }
                }
            }
            imported_by_root.push((root.clone(), raw_count, expanded));
        }
        imported_by_root
    };

    // Phase 2: Filesystem scan
    let (scanned_files, imported_count, all_orphans, scan_errors) =
        tokio::task::spawn_blocking(move || {
            let mut total_disk_files = 0usize;
            let mut total_imported = 0usize;
            let mut orphans: Vec<(String, u64)> = Vec::new();
            let mut errors: Vec<(String, String)> = Vec::new();

            for (root, raw_count, imported) in &imported_by_root {
                let disk_files = match scan_audio_directory(root, true, None) {
                    Ok(f) => f,
                    Err(e) => {
                        errors.push((root.clone(), e));
                        continue;
                    }
                };
                total_disk_files += disk_files.len();
                total_imported += raw_count;

                for file_path in &disk_files {
                    if !imported.contains(file_path) {
                        let size = std::fs::metadata(file_path).map(|m| m.len()).unwrap_or(0);
                        orphans.push((file_path.clone(), size));
                    }
                }
            }

            (total_disk_files, total_imported, orphans, errors)
        })
        .await
        .map_err(|e| mcp_internal_error(format!("Task join error: {e}")))?;

    let orphan_count = all_orphans.len();

    let mut by_dir: BTreeMap<String, Vec<serde_json::Value>> = BTreeMap::new();
    for (reported, (path, size)) in all_orphans.iter().enumerate() {
        if reported >= limit {
            break;
        }
        let parent = std::path::Path::new(path)
            .parent()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let filename = std::path::Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        by_dir.entry(parent).or_default().push(serde_json::json!({
            "filename": filename,
            "size_bytes": size,
        }));
    }

    let mut result = serde_json::json!({
        "scanned_files": scanned_files,
        "imported_count": imported_count,
        "orphan_count": orphan_count,
        "orphans_by_directory": by_dir,
    });

    if !scan_errors.is_empty() {
        result["scan_errors"] = serde_json::json!(
            scan_errors
                .iter()
                .map(|(root, err)| serde_json::json!({ "root": root, "error": err }))
                .collect::<Vec<_>>()
        );
    }

    ok_json(&result)
}

pub(super) async fn handle_scan_duplicates(
    server: &ReklawdboxServer,
    params: ScanDuplicatesParams,
) -> Result<CallToolResult, McpError> {
    match params.detection_level.unwrap_or_default() {
        DuplicateDetectionLevel::Metadata => handle_duplicates_metadata(server, params).await,
        DuplicateDetectionLevel::Exact => handle_duplicates_exact(server, params).await,
    }
}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub(super) struct ScanDuplicatesOutput {
    detection_level: String,
    group_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    total_groups_found: Option<usize>,
    total_duplicate_tracks: usize,
    duplicate_groups: Vec<serde_json::Value>,
    page: OffsetPage,
    #[serde(skip_serializing_if = "Option::is_none")]
    hash_errors_count: Option<usize>,
}

fn sort_exact_duplicate_groups(groups: &mut [(String, Vec<String>)]) {
    for (_, ids) in groups.iter_mut() {
        ids.sort();
    }
    groups.sort_by(|(left_hash, left_ids), (right_hash, right_ids)| {
        right_ids
            .len()
            .cmp(&left_ids.len())
            .then_with(|| left_hash.cmp(right_hash))
    });
}

async fn handle_duplicates_metadata(
    server: &ReklawdboxServer,
    params: ScanDuplicatesParams,
) -> Result<CallToolResult, McpError> {
    let conn = server.rekordbox_conn()?;
    let limit = params.limit.unwrap_or(50).min(200);
    let offset = params.offset.unwrap_or(0);
    let duplicate_page =
        db::find_metadata_duplicates_page(&conn, params.path_prefix.as_deref(), limit, offset)
            .map_err(db_error)?;
    let groups = duplicate_page.groups;
    let (_, mut page) = offset_page_bounds(duplicate_page.total, offset as usize, limit as usize);
    page.returned = groups.len();

    let all_ids: Vec<String> = groups
        .iter()
        .flat_map(|g| g.track_ids.iter().cloned())
        .collect();
    let tracks = db::get_tracks_by_ids(&conn, &all_ids).map_err(db_error)?;
    let memberships = db::playlist_membership_counts(&conn, &all_ids).map_err(db_error)?;
    drop(conn);

    let track_map: HashMap<&str, &crate::types::Track> =
        tracks.iter().map(|t| (t.id.as_str(), t)).collect();

    let mut total_dup_tracks = 0usize;
    let mut dup_groups = Vec::new();

    for group in &groups {
        let group_tracks: Vec<&crate::types::Track> = group
            .track_ids
            .iter()
            .filter_map(|id| track_map.get(id.as_str()).copied())
            .collect();
        if group_tracks.len() < 2 {
            continue;
        }
        total_dup_tracks += group_tracks.len();
        let suggested = pick_suggested_keep(&group_tracks);
        let entries: Vec<serde_json::Value> = group_tracks
            .iter()
            .map(|t| track_to_dup_entry(t, memberships.get(&t.id).copied().unwrap_or(0)))
            .collect();
        dup_groups.push(serde_json::json!({
            "key": format!("{} - {}", group.artist, group.title),
            "suggested_keep": suggested,
            "tracks": entries,
        }));
    }

    page.returned = dup_groups.len();
    ok_structured_json(ScanDuplicatesOutput {
        detection_level: "metadata".to_string(),
        group_count: dup_groups.len(),
        total_groups_found: None,
        total_duplicate_tracks: total_dup_tracks,
        duplicate_groups: dup_groups,
        page,
        hash_errors_count: None,
    })
}

async fn handle_duplicates_exact(
    server: &ReklawdboxServer,
    params: ScanDuplicatesParams,
) -> Result<CallToolResult, McpError> {
    let limit = params.limit.unwrap_or(50).min(200) as usize;
    let offset = params.offset.unwrap_or(0) as usize;
    // Scoped to drop conn before await
    let track_paths = {
        let conn = server.rekordbox_conn()?;
        db::all_track_paths(&conn, params.path_prefix.as_deref()).map_err(db_error)?
    };

    let size_groups: HashMap<u64, Vec<(String, String)>> = tokio::task::spawn_blocking(move || {
        let mut sizes: HashMap<u64, Vec<(String, String)>> = HashMap::new();
        for entry in &track_paths {
            if let Ok(resolved) = audio::resolve_audio_path(&entry.path)
                && let Ok(meta) = std::fs::metadata(&resolved)
            {
                sizes
                    .entry(meta.len())
                    .or_default()
                    .push((resolved, entry.id.clone()));
            }
        }
        sizes.retain(|_, v| v.len() > 1);
        sizes
    })
    .await
    .map_err(|e| mcp_internal_error(format!("Task join error: {e}")))?;

    if size_groups.is_empty() {
        let (_, page) = offset_page_bounds(0, offset, limit);
        return ok_structured_json(ScanDuplicatesOutput {
            detection_level: "exact".to_string(),
            group_count: 0,
            total_groups_found: Some(0),
            total_duplicate_tracks: 0,
            duplicate_groups: Vec::new(),
            page,
            hash_errors_count: None,
        });
    }

    let sem = Arc::new(tokio::sync::Semaphore::new(8));
    let mut handles = Vec::new();

    for entries in size_groups.values() {
        for (path, track_id) in entries {
            let sem = sem.clone();
            let path = path.clone();
            let tid = track_id.clone();
            handles.push(tokio::spawn(async move {
                let _permit = sem.acquire().await.expect("semaphore closed");
                let hash = tokio::task::spawn_blocking(move || hash_file(&path))
                    .await
                    .unwrap_or_else(|e| Err(std::io::Error::other(e)));
                (tid, hash)
            }));
        }
    }

    let mut hash_groups: HashMap<String, Vec<String>> = HashMap::new();
    let mut hash_errors_count = 0usize;
    for handle in handles {
        let (track_id, hash_result) = handle
            .await
            .map_err(|e| mcp_internal_error(format!("Task join error: {e}")))?;
        match hash_result {
            Ok(hash) => {
                hash_groups.entry(hash).or_default().push(track_id);
            }
            Err(_) => {
                hash_errors_count += 1;
            }
        }
    }
    hash_groups.retain(|_, v| v.len() > 1);

    let mut ordered_hash_groups: Vec<_> = hash_groups.into_iter().collect();
    sort_exact_duplicate_groups(&mut ordered_hash_groups);
    let total_groups_found = ordered_hash_groups.len();
    let (group_range, mut page) = offset_page_bounds(total_groups_found, offset, limit);
    let groups_to_report: Vec<_> = ordered_hash_groups[group_range].to_vec();

    if groups_to_report.is_empty() {
        return ok_structured_json(ScanDuplicatesOutput {
            detection_level: "exact".to_string(),
            group_count: 0,
            total_groups_found: Some(total_groups_found),
            total_duplicate_tracks: 0,
            duplicate_groups: Vec::new(),
            page,
            hash_errors_count: (hash_errors_count > 0).then_some(hash_errors_count),
        });
    }

    let all_ids: Vec<String> = groups_to_report
        .iter()
        .flat_map(|(_, ids)| ids.iter().cloned())
        .collect();
    let (tracks, memberships) = {
        let conn = server.rekordbox_conn()?;
        let tracks = db::get_tracks_by_ids(&conn, &all_ids).map_err(db_error)?;
        let memberships = db::playlist_membership_counts(&conn, &all_ids).map_err(db_error)?;
        (tracks, memberships)
    };

    let track_map: HashMap<&str, &crate::types::Track> =
        tracks.iter().map(|t| (t.id.as_str(), t)).collect();

    let mut total_dup_tracks = 0usize;
    let mut dup_groups = Vec::new();

    for (hash, ids) in &groups_to_report {
        let group_tracks: Vec<&crate::types::Track> = ids
            .iter()
            .filter_map(|id| track_map.get(id.as_str()).copied())
            .collect();
        if group_tracks.len() < 2 {
            continue;
        }
        total_dup_tracks += group_tracks.len();
        let suggested = pick_suggested_keep(&group_tracks);
        let entries: Vec<serde_json::Value> = group_tracks
            .iter()
            .map(|t| track_to_dup_entry(t, memberships.get(&t.id).copied().unwrap_or(0)))
            .collect();
        dup_groups.push(serde_json::json!({
            "key": format!("sha256:{hash}"),
            "suggested_keep": suggested,
            "tracks": entries,
        }));
    }

    page.returned = dup_groups.len();
    ok_structured_json(ScanDuplicatesOutput {
        detection_level: "exact".to_string(),
        group_count: dup_groups.len(),
        total_groups_found: Some(total_groups_found),
        total_duplicate_tracks: total_dup_tracks,
        duplicate_groups: dup_groups,
        page,
        hash_errors_count: (hash_errors_count > 0).then_some(hash_errors_count),
    })
}

fn hash_file(path: &str) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn pick_suggested_keep(tracks: &[&crate::types::Track]) -> Option<String> {
    tracks
        .iter()
        .max_by(|a, b| {
            a.bit_rate
                .cmp(&b.bit_rate)
                .then(a.sample_rate.cmp(&b.sample_rate))
                .then(a.play_count.cmp(&b.play_count))
                .then(a.rating.cmp(&b.rating))
        })
        .map(|t| t.id.clone())
}

fn track_to_dup_entry(track: &crate::types::Track, in_playlists: i32) -> serde_json::Value {
    serde_json::json!({
        "track_id": track.id,
        "artist": track.artist,
        "title": track.title,
        "file_path": track.file_path,
        "bitrate": track.bit_rate,
        "sample_rate": track.sample_rate,
        "file_type": track.file_kind,
        "rating": track.rating,
        "play_count": track.play_count,
        "in_playlists": in_playlists,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{FileKind, Track};

    fn make_track(id: &str, bit_rate: i32, sample_rate: i32, play_count: i32, rating: u8) -> Track {
        Track {
            id: id.to_string(),
            title: String::new(),
            artist: String::new(),
            album: String::new(),
            genre: String::new(),
            bpm: 0.0,
            key: String::new(),
            rating,
            comments: String::new(),
            color: String::new(),
            color_code: 0,
            label: String::new(),
            remixer: String::new(),
            year: 0,
            length: 0,
            file_path: String::new(),
            play_count,
            bit_rate,
            sample_rate,
            file_kind: FileKind::Unknown(0),
            date_added: String::new(),
            position: None,
            played_at: None,
        }
    }

    #[test]
    fn test_pick_suggested_keep_prefers_higher_bitrate() {
        let low = make_track("low", 320, 44100, 10, 5);
        let high = make_track("high", 1411, 44100, 0, 0);
        assert_eq!(
            pick_suggested_keep(&[&low, &high]),
            Some("high".to_string())
        );
    }

    #[test]
    fn test_pick_suggested_keep_cascades_tiebreakers() {
        let a = make_track("a", 1411, 44100, 5, 3);
        let b = make_track("b", 1411, 44100, 5, 4); // same except higher rating
        assert_eq!(pick_suggested_keep(&[&a, &b]), Some("b".to_string()));
    }

    #[test]
    fn test_pick_suggested_keep_empty() {
        let result: Option<String> = pick_suggested_keep(&[]);
        assert_eq!(result, None);
    }

    #[test]
    fn scan_duplicates_page_exact_order_is_stable_and_non_overlapping() {
        let mut groups = vec![
            (
                "hash-z".to_string(),
                vec!["t4".to_string(), "t3".to_string()],
            ),
            (
                "hash-a".to_string(),
                vec!["t2".to_string(), "t1".to_string(), "t0".to_string()],
            ),
            (
                "hash-b".to_string(),
                vec!["t6".to_string(), "t5".to_string()],
            ),
        ];
        sort_exact_duplicate_groups(&mut groups);
        assert_eq!(
            groups
                .iter()
                .map(|(hash, _)| hash.as_str())
                .collect::<Vec<_>>(),
            ["hash-a", "hash-b", "hash-z"]
        );
        assert_eq!(groups[0].1, ["t0", "t1", "t2"]);

        let (first, first_page) = offset_page_bounds(groups.len(), 0, 2);
        let (second, second_page) = offset_page_bounds(groups.len(), 2, 2);
        assert_eq!(first, 0..2);
        assert_eq!(second, 2..3);
        assert_eq!(first_page.next_offset, Some(2));
        assert!(first_page.has_more);
        assert_eq!(second_page.next_offset, None);
        assert!(!second_page.has_more);
    }

    #[test]
    fn scan_duplicates_page_zero_and_beyond_end_are_terminal() {
        let (zero, zero_page) = offset_page_bounds(5, 2, 0);
        assert_eq!(zero, 2..2);
        assert_eq!(zero_page.returned, 0);
        assert_eq!(zero_page.next_offset, None);
        assert!(!zero_page.has_more);

        let (beyond, beyond_page) = offset_page_bounds(5, usize::MAX, 50);
        assert_eq!(beyond, 5..5);
        assert_eq!(beyond_page.returned, 0);
        assert_eq!(beyond_page.next_offset, None);
        assert!(!beyond_page.has_more);
    }
}
