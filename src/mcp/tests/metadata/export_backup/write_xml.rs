use super::supervision::{
    fixture_pid, wait_for_pid_exit, write_early_exit_backup_fixture, write_hanging_backup_fixture,
};
use super::support::{EnvVarGuard, backup_script_env_lock, write_executable_script};
use crate::mcp::metadata::{
    TrackChangeInput, UpdateTracksParams, WriteXmlParams, WriteXmlPlaylistInput,
};
use crate::mcp::server::ReklawdboxServer;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use rmcp::ErrorData as McpError;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;

use crate::adapters::state as store;
use crate::domain::metadata::TrackChange;

use super::super::super::common::{
    call_tool_via_router, create_server_with_connections, create_single_track_test_db,
    default_http_client_for_tests, extract_json, insert_test_track,
};

#[tokio::test]
async fn write_xml_no_change_path_returns_message() {
    let server = ReklawdboxServer::new(None);

    let result = server
        .write_xml(Parameters(WriteXmlParams {
            skip_label_gate: Some(true),
            output_path: None,
            playlists: None,
        }))
        .await
        .expect("write_xml should succeed when no changes are staged");

    let payload = extract_json(&result);
    assert_eq!(
        payload
            .get("message")
            .and_then(serde_json::Value::as_str)
            .expect("message should be present"),
        "No changes to write."
    );
}

#[tokio::test]
async fn write_xml_no_change_path_via_router_returns_message() {
    let result = call_tool_via_router("write_xml", None).await;
    let payload = extract_json(&result);

    assert_eq!(
        payload
            .get("message")
            .and_then(serde_json::Value::as_str)
            .expect("message should be present"),
        "No changes to write."
    );
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn write_xml_serializes_overlapping_exports() {
    let _env_guard = backup_script_env_lock()
        .lock()
        .expect("backup env mutex should not be poisoned");

    let db_conn = create_single_track_test_db("overlap-track-1", "/tmp/overlap-track-1.flac");
    let store_dir = tempfile::tempdir().expect("temp store dir should create");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(
        store_path
            .to_str()
            .expect("temp store path should be UTF-8"),
    )
    .expect("temp internal store should open");
    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());
    server.context.mutation.changes.stage(vec![TrackChange {
        track_id: "overlap-track-1".to_string(),
        genre: Some("Techno".to_string()),
        ..Default::default()
    }]);

    let output_dir = tempfile::tempdir().expect("temp output dir should create");
    let first_path = output_dir.path().join("first.xml");
    let second_path = output_dir.path().join("second.xml");
    let mut tasks = WriteXmlTaskCleanup::new();
    let mut held_lock = Some(
        tokio::time::timeout(
            WRITE_XML_TASK_TIMEOUT,
            server.context.mutation.xml_export_lock.lock(),
        )
        .await
        .expect("test should acquire export lock within five seconds"),
    );

    let scenario = async {
        let first_queued = Arc::new(tokio::sync::Notify::new());
        tasks.push(spawn_queued_write_xml(
            server.clone(),
            WriteXmlParams {
                skip_label_gate: Some(true),
                output_path: Some(first_path.to_string_lossy().to_string()),
                playlists: None,
            },
            Arc::clone(&first_queued),
        ));
        wait_for_queued_write_xml(&first_queued, "first export queue").await?;

        let second_queued = Arc::new(tokio::sync::Notify::new());
        tasks.push(spawn_queued_write_xml(
            server.clone(),
            WriteXmlParams {
                skip_label_gate: Some(true),
                output_path: Some(second_path.to_string_lossy().to_string()),
                playlists: None,
            },
            Arc::clone(&second_queued),
        ));
        wait_for_queued_write_xml(&second_queued, "second export queue").await?;

        if !tasks.all_pending() {
            return Err("queued exports should remain pending while the lock is held".to_string());
        }
        if server.context.mutation.changes.pending_count() != 1 {
            return Err(
                "queued exports crossed the take boundary while the lock was held".to_string(),
            );
        }

        drop(held_lock.take());
        let first = tasks
            .join(0, "first overlapping export")
            .await?
            .map_err(|err| format!("first overlapping export failed: {err:?}"))?;
        let second = tasks
            .join(1, "second overlapping export")
            .await?
            .map_err(|err| format!("second overlapping export failed: {err:?}"))?;
        Ok::<_, String>((extract_json(&first), extract_json(&second)))
    };

    let scenario_result = tokio::time::timeout(WRITE_XML_TASK_TIMEOUT, scenario).await;
    let (first, second) = match scenario_result {
        Ok(Ok(payloads)) => payloads,
        Ok(Err(err)) => {
            drop(held_lock.take());
            let cleanup = tasks.abort_all().await;
            panic!("overlapping export scenario failed: {err}; cleanup: {cleanup:?}");
        }
        Err(_) => {
            drop(held_lock.take());
            let cleanup = tasks.abort_all().await;
            panic!("overlapping export scenario timed out; cleanup: {cleanup:?}");
        }
    };

    let applied = [
        first["changes_applied"].as_u64().unwrap_or_default(),
        second["changes_applied"].as_u64().unwrap_or_default(),
    ];
    assert_eq!(applied.iter().sum::<u64>(), 1);
    assert_eq!(applied.iter().filter(|&&count| count == 1).count(), 1);
    assert_eq!(applied.iter().filter(|&&count| count == 0).count(), 1);
    assert_eq!(server.context.mutation.changes.pending_count(), 0);
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn write_xml_cancelled_waiter_does_not_touch_snapshots() {
    let _env_guard = backup_script_env_lock()
        .lock()
        .expect("backup env mutex should not be poisoned");

    let db_conn = create_single_track_test_db("cancel-track-1", "/tmp/cancel-track-1.flac");
    let store_dir = tempfile::tempdir().expect("temp store dir should create");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(
        store_path
            .to_str()
            .expect("temp store path should be UTF-8"),
    )
    .expect("temp internal store should open");
    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());
    server.context.mutation.changes.stage(vec![TrackChange {
        track_id: "cancel-track-1".to_string(),
        genre: Some("Techno".to_string()),
        ..Default::default()
    }]);

    let output_dir = tempfile::tempdir().expect("temp output dir should create");
    let active_path = output_dir.path().join("active.xml");
    let cancelled_path = output_dir.path().join("cancelled.xml");
    let next_path = output_dir.path().join("next.xml");
    let mut tasks = WriteXmlTaskCleanup::new();
    let mut held_lock = Some(
        tokio::time::timeout(
            WRITE_XML_TASK_TIMEOUT,
            server.context.mutation.xml_export_lock.lock(),
        )
        .await
        .expect("test should acquire export lock within five seconds"),
    );

    let scenario = async {
        let active_queued = Arc::new(tokio::sync::Notify::new());
        tasks.push(spawn_queued_write_xml(
            server.clone(),
            WriteXmlParams {
                skip_label_gate: Some(true),
                output_path: Some(active_path.to_string_lossy().to_string()),
                playlists: None,
            },
            Arc::clone(&active_queued),
        ));
        wait_for_queued_write_xml(&active_queued, "active export queue").await?;

        let cancelled_queued = Arc::new(tokio::sync::Notify::new());
        tasks.push(spawn_queued_write_xml(
            server.clone(),
            WriteXmlParams {
                skip_label_gate: Some(true),
                output_path: Some(cancelled_path.to_string_lossy().to_string()),
                playlists: None,
            },
            Arc::clone(&cancelled_queued),
        ));
        wait_for_queued_write_xml(&cancelled_queued, "cancelled export queue").await?;

        if !tasks.all_pending() || server.context.mutation.changes.pending_count() != 1 {
            return Err("both exports should remain before the take boundary".to_string());
        }
        tasks.abort(1, "cancelled queued export").await?;
        if server.context.mutation.changes.pending_count() != 1 {
            return Err("cancelling a waiter changed the staged snapshot".to_string());
        }

        drop(held_lock.take());
        let active = tasks
            .join(0, "active export after waiter cancellation")
            .await?
            .map_err(|err| format!("active export failed: {err:?}"))?;
        if server.context.mutation.changes.pending_count() != 0 {
            return Err("active export should commit its snapshot exactly once".to_string());
        }

        server.context.mutation.changes.stage(vec![TrackChange {
            track_id: "cancel-track-1".to_string(),
            genre: Some("Trance".to_string()),
            ..Default::default()
        }]);
        let next = tokio::time::timeout(
            WRITE_XML_TASK_TIMEOUT,
            server.write_xml(Parameters(WriteXmlParams {
                skip_label_gate: Some(true),
                output_path: Some(next_path.to_string_lossy().to_string()),
                playlists: None,
            })),
        )
        .await
        .map_err(|_| "next export did not finish within five seconds".to_string())?
        .map_err(|err| format!("next export failed: {err:?}"))?;

        Ok::<_, String>((extract_json(&active), extract_json(&next)))
    };

    let scenario_result = tokio::time::timeout(WRITE_XML_TASK_TIMEOUT, scenario).await;
    let (active, next) = match scenario_result {
        Ok(Ok(payloads)) => payloads,
        Ok(Err(err)) => {
            drop(held_lock.take());
            let cleanup = tasks.abort_all().await;
            panic!("cancelled waiter scenario failed: {err}; cleanup: {cleanup:?}");
        }
        Err(_) => {
            drop(held_lock.take());
            let cleanup = tasks.abort_all().await;
            panic!("cancelled waiter scenario timed out; cleanup: {cleanup:?}");
        }
    };

    assert_eq!(active["changes_applied"], 1);
    assert_eq!(next["changes_applied"], 1);
    assert!(!cancelled_path.exists());
    assert_eq!(server.context.mutation.changes.pending_count(), 0);
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn write_xml_with_playlists_exports_without_staged_changes() {
    let _env_guard = backup_script_env_lock()
        .lock()
        .expect("backup env mutex should not be poisoned");

    let db_conn = create_single_track_test_db("playlist-track-1", "/tmp/playlist-track-1.flac");
    let store_dir = tempfile::tempdir().expect("temp store dir should create");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(
        store_path
            .to_str()
            .expect("temp store path should be UTF-8"),
    )
    .expect("temp internal store should open");
    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());

    let output_dir = tempfile::tempdir().expect("temp output dir should create");
    let output_path = output_dir.path().join("playlist-export.xml");
    let output_path_str = output_path.to_string_lossy().to_string();

    let result = server
        .write_xml(Parameters(WriteXmlParams {
            skip_label_gate: Some(true),
            output_path: Some(output_path_str.clone()),
            playlists: Some(vec![WriteXmlPlaylistInput {
                name: "Set & Test".to_string(),
                track_ids: vec!["playlist-track-1".to_string()],
            }]),
        }))
        .await
        .expect("write_xml should export playlist-only requests");

    let payload = extract_json(&result);
    assert_eq!(payload["track_count"], 1);
    assert_eq!(payload["changes_applied"], 0);
    assert_eq!(payload["playlist_count"], 1);
    assert_eq!(
        payload["path"].as_str().expect("path should be present"),
        output_path_str
    );

    let xml = std::fs::read_to_string(&output_path).expect("XML output should be readable");
    assert!(xml.contains("<PLAYLISTS>"));
    assert!(xml.contains("Name=\"Set &amp; Test\""));
    assert!(xml.contains("<TRACK Key=\"1\"/>"));
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn write_xml_with_playlists_reports_missing_track_ids() {
    let _env_guard = backup_script_env_lock()
        .lock()
        .expect("backup env mutex should not be poisoned");

    let db_conn = create_single_track_test_db("playlist-track-1", "/tmp/playlist-track-1.flac");
    let store_dir = tempfile::tempdir().expect("temp store dir should create");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(
        store_path
            .to_str()
            .expect("temp store path should be UTF-8"),
    )
    .expect("temp internal store should open");
    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());

    let err = server
        .write_xml(Parameters(WriteXmlParams {
            skip_label_gate: Some(true),
            output_path: None,
            playlists: Some(vec![WriteXmlPlaylistInput {
                name: "Bad Set".to_string(),
                track_ids: vec!["does-not-exist".to_string()],
            }]),
        }))
        .await
        .expect_err("missing playlist track IDs should fail");

    let msg = format!("{err:?}");
    assert!(msg.contains("Track IDs not found in database"));
    assert!(msg.contains("does-not-exist"));
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn write_xml_label_gate_blocks_when_set() {
    let _env_guard = backup_script_env_lock()
        .lock()
        .expect("backup env mutex should not be poisoned");

    let db_conn = create_single_track_test_db("gate-track-1", "/tmp/gate-track-1.flac");
    let store_dir = tempfile::tempdir().expect("temp store dir should create");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(
        store_path
            .to_str()
            .expect("temp store path should be UTF-8"),
    )
    .expect("temp internal store should open");
    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());

    server.context.mutation.changes.stage(vec![TrackChange {
        track_id: "gate-track-1".to_string(),
        genre: None,
        comments: None,
        rating: None,
        color: None,
        label: Some("Test Label".to_string()),
        year: None,
        album: None,
    }]);

    server
        .context
        .mutation
        .label_research_gate
        .store(50, std::sync::atomic::Ordering::Relaxed);

    let err = server
        .write_xml(Parameters(WriteXmlParams {
            skip_label_gate: None,
            output_path: None,
            playlists: None,
        }))
        .await
        .expect_err("label gate should block write_xml");

    let msg = format!("{err:?}");
    assert!(
        msg.contains("Label research gate"),
        "error should mention label research gate, got: {msg}"
    );
    assert!(
        msg.contains("50"),
        "error should mention the unlabeled count, got: {msg}"
    );

    let result = server
        .write_xml(Parameters(WriteXmlParams {
            skip_label_gate: Some(true),
            output_path: None,
            playlists: None,
        }))
        .await
        .expect("skip_label_gate=true should bypass the gate");

    let payload = extract_json(&result);
    assert!(payload.get("track_count").is_some());
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn write_xml_label_gate_clears_when_zero() {
    let _env_guard = backup_script_env_lock()
        .lock()
        .expect("backup env mutex should not be poisoned");

    let db_conn = create_single_track_test_db("gate-clear-1", "/tmp/gate-clear-1.flac");
    let store_dir = tempfile::tempdir().expect("temp store dir should create");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(
        store_path
            .to_str()
            .expect("temp store path should be UTF-8"),
    )
    .expect("temp internal store should open");
    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());

    server
        .context
        .mutation
        .label_research_gate
        .store(50, std::sync::atomic::Ordering::Relaxed);
    server
        .context
        .mutation
        .label_research_gate
        .store(0, std::sync::atomic::Ordering::Relaxed);

    server.context.mutation.changes.stage(vec![TrackChange {
        track_id: "gate-clear-1".to_string(),
        genre: None,
        comments: None,
        rating: None,
        color: None,
        label: Some("Test".to_string()),
        year: None,
        album: None,
    }]);

    let result = server
        .write_xml(Parameters(WriteXmlParams {
            skip_label_gate: None,
            output_path: None,
            playlists: None,
        }))
        .await
        .expect("gate=0 should not block write_xml");

    let payload = extract_json(&result);
    assert!(payload.get("track_count").is_some());
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn write_xml_deduplicates_playlist_and_staged_tracks() {
    let _env_guard = backup_script_env_lock()
        .lock()
        .expect("backup env mutex should not be poisoned");

    let db_conn = create_single_track_test_db("staged-track-1", "/tmp/staged-track-1.flac");
    insert_test_track(
        &db_conn,
        "playlist-track-2",
        "Playlist Only",
        "g1",
        "/tmp/playlist-track-2.flac",
    );

    let store_dir = tempfile::tempdir().expect("temp store dir should create");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(
        store_path
            .to_str()
            .expect("temp store path should be UTF-8"),
    )
    .expect("temp internal store should open");
    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());

    server
        .update_tracks(Parameters(UpdateTracksParams {
            changes: vec![TrackChangeInput {
                track_id: "staged-track-1".to_string(),
                genre: None,
                comments: Some("staged only comment".to_string()),
                rating: Some(5),
                color: None,
                label: None,
                year: None,
                album: None,
            }],
        }))
        .await
        .expect("staging update should succeed");

    let output_dir = tempfile::tempdir().expect("temp output dir should create");
    let output_path = output_dir.path().join("mixed-export.xml");
    let output_path_str = output_path.to_string_lossy().to_string();

    let result = server
        .write_xml(Parameters(WriteXmlParams {
            skip_label_gate: Some(true),
            output_path: Some(output_path_str.clone()),
            playlists: Some(vec![WriteXmlPlaylistInput {
                name: "Mixed Export".to_string(),
                track_ids: vec!["playlist-track-2".to_string(), "staged-track-1".to_string()],
            }]),
        }))
        .await
        .expect("write_xml should succeed for mixed staged + playlist exports");

    let payload = extract_json(&result);
    assert_eq!(payload["track_count"], 2);
    assert_eq!(payload["changes_applied"], 1);
    assert_eq!(payload["playlist_count"], 1);
    assert_eq!(
        payload["path"].as_str().expect("path should be present"),
        output_path_str
    );

    let xml = std::fs::read_to_string(&output_path).expect("XML output should be readable");
    assert!(xml.contains("<COLLECTION Entries=\"2\">"));
    assert_eq!(xml.matches("TrackID=\"").count(), 2);
    assert_eq!(xml.matches("Name=\"Señorita\"").count(), 1);
    assert_eq!(xml.matches("Name=\"Playlist Only\"").count(), 1);

    let staged_line = xml
        .lines()
        .find(|line| line.contains("Name=\"Señorita\""))
        .expect("staged track line should exist");
    assert!(
        staged_line.contains("Comments=\"staged only comment\""),
        "staged comment should be applied to staged track"
    );
    assert!(
        staged_line.contains("Rating=\"255\""),
        "5-star staged rating should be encoded as 255"
    );

    let playlist_only_line = xml
        .lines()
        .find(|line| line.contains("Name=\"Playlist Only\""))
        .expect("playlist-only track line should exist");
    assert!(
        playlist_only_line.contains("Comments=\"cache coverage test\""),
        "playlist-only track should keep DB comments when no staged changes exist"
    );
    assert!(
        playlist_only_line.contains("Rating=\"102\""),
        "playlist-only track should keep DB-derived rating when not staged"
    );

    let playlist_line = xml
        .lines()
        .find(|line| {
            line.contains("<NODE")
                && line.contains("Type=\"1\"")
                && line.contains("Name=\"Mixed Export\"")
                && line.contains("Entries=\"2\"")
                && line.contains("KeyType=\"0\"")
        })
        .expect("playlist node should exist with expected attributes");
    let playlist_start = xml
        .find(playlist_line)
        .expect("playlist line should be findable in xml");
    let playlist_end = playlist_start
        + xml[playlist_start..]
            .find("</NODE>")
            .expect("playlist node should close");
    let playlist_block = &xml[playlist_start..playlist_end];
    let key2 = playlist_block
        .find("<TRACK Key=\"2\"/>")
        .expect("playlist should reference playlist-only track");
    let key1 = playlist_block
        .find("<TRACK Key=\"1\"/>")
        .expect("playlist should reference staged track");
    assert!(
        key2 < key1,
        "playlist key order should follow input track_ids order"
    );
}

#[tokio::test]
#[cfg(unix)]
#[allow(clippy::await_holding_lock)]
async fn write_xml_fails_closed_when_backup_script_fails_and_restores_changes() {
    use std::os::unix::fs::PermissionsExt;

    let _env_guard = backup_script_env_lock()
        .lock()
        .expect("backup env mutex should not be poisoned");

    let db_conn = create_single_track_test_db("staged-track-1", "/tmp/staged-track-1.flac");
    let store_dir = tempfile::tempdir().expect("temp store dir should create");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(
        store_path
            .to_str()
            .expect("temp store path should be UTF-8"),
    )
    .expect("temp internal store should open");
    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());

    server
        .update_tracks(Parameters(UpdateTracksParams {
            changes: vec![TrackChangeInput {
                track_id: "staged-track-1".to_string(),
                genre: Some("Techno".to_string()),
                comments: None,
                rating: None,
                color: None,
                label: None,
                year: None,
                album: None,
            }],
        }))
        .await
        .expect("staging update should succeed");

    let backup_dir = tempfile::tempdir().expect("temp backup dir should create");
    let backup_script = backup_dir.path().join("fail-backup.sh");
    std::fs::write(
        &backup_script,
        "#!/bin/sh\necho 'backup failed intentionally' >&2\nexit 23\n",
    )
    .expect("backup script should be written");
    let mut perms = std::fs::metadata(&backup_script)
        .expect("backup script metadata should be readable")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&backup_script, perms).expect("backup script should be executable");

    let _backup_script_env = EnvVarGuard::set("REKLAWDBOX_BACKUP_SCRIPT", &backup_script);

    let output_dir = tempfile::tempdir().expect("temp output dir should create");
    let output_path = output_dir.path().join("should-not-exist.xml");
    let err = server
        .write_xml(Parameters(WriteXmlParams {
            skip_label_gate: Some(true),
            output_path: Some(output_path.to_string_lossy().to_string()),
            playlists: None,
        }))
        .await
        .expect_err("write_xml should fail when backup script fails");

    let msg = format!("{err:?}");
    assert!(msg.contains("pre-op backup failed with exit status 23"));
    assert!(msg.contains("backup failed intentionally"));
    assert!(
        !output_path.exists(),
        "XML export should not be written after backup failure"
    );

    drop(_backup_script_env);

    let retry_path = output_dir.path().join("after-backup-failure.xml");
    let retry = server
        .write_xml(Parameters(WriteXmlParams {
            skip_label_gate: Some(true),
            output_path: Some(retry_path.to_string_lossy().to_string()),
            playlists: None,
        }))
        .await
        .expect("staged changes should be restored after backup failure");
    let payload = extract_json(&retry);
    assert_eq!(payload["changes_applied"], 1);

    let xml = std::fs::read_to_string(&retry_path).expect("retry XML output should be readable");
    assert!(
        xml.contains("Genre=\"Techno\""),
        "restored staged change should still be exported on retry"
    );
}

#[cfg(unix)]
#[derive(Clone, Copy)]
enum WriteXmlBackupFailure {
    HungParent,
    EarlyExitDescendant,
}

#[cfg(unix)]
#[allow(clippy::await_holding_lock)]
async fn assert_write_xml_backup_failure_restores_state(failure: WriteXmlBackupFailure) {
    let _env_guard = backup_script_env_lock()
        .lock()
        .expect("backup env mutex should not be poisoned");
    let db_conn =
        create_single_track_test_db("backup-timeout-track", "/tmp/backup-timeout-track.flac");
    let store_dir = tempfile::tempdir().expect("temp store dir should create");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(
        store_path
            .to_str()
            .expect("temp store path should be UTF-8"),
    )
    .expect("temp internal store should open");
    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());
    server.context.mutation.changes.stage(vec![TrackChange {
        track_id: "backup-timeout-track".to_string(),
        genre: Some("Techno".to_string()),
        comments: Some("restored after timeout".to_string()),
        rating: Some(5),
        ..Default::default()
    }]);
    let staged_before = serde_json::to_value(
        server
            .context
            .mutation
            .changes
            .get("backup-timeout-track")
            .expect("staged change should exist before export"),
    )
    .expect("staged change should serialize");

    let backup_dir = tempfile::tempdir().expect("hanging backup fixture should create");
    let fixture = match failure {
        WriteXmlBackupFailure::HungParent => write_hanging_backup_fixture(backup_dir.path(), true),
        WriteXmlBackupFailure::EarlyExitDescendant => {
            write_early_exit_backup_fixture(backup_dir.path())
        }
    };
    let _script_env = EnvVarGuard::set("REKLAWDBOX_BACKUP_SCRIPT", &fixture.script);
    let timeout_override =
        crate::adapters::rekordbox::backup::override_pre_op_backup_timeout_for_test(
            Duration::from_millis(250),
        );
    let output_dir = tempfile::tempdir().expect("temp output dir should create");
    let timed_out_path = output_dir.path().join("must-not-exist.xml");

    let error = tokio::time::timeout(
        Duration::from_secs(5),
        server.write_xml(Parameters(WriteXmlParams {
            skip_label_gate: Some(true),
            output_path: Some(timed_out_path.to_string_lossy().to_string()),
            playlists: None,
        })),
    )
    .await
    .expect("timed-out write_xml should return within five seconds")
    .expect_err("timed-out backup must fail XML export");
    let expected_error = match failure {
        WriteXmlBackupFailure::HungParent => "pre-op pre-operation backup timed out after 250ms",
        WriteXmlBackupFailure::EarlyExitDescendant => {
            "pre-op backup script exited while descendant processes were still running"
        }
    };
    assert!(
        error.message.contains(expected_error),
        "unexpected backup failure: {}",
        error.message
    );
    assert!(!timed_out_path.exists(), "timed-out XML must not exist");
    assert_eq!(
        std::fs::read_dir(output_dir.path())
            .expect("output directory should be readable")
            .count(),
        0,
        "timed-out export must leave no target or temporary output"
    );
    let staged_after = serde_json::to_value(
        server
            .context
            .mutation
            .changes
            .get("backup-timeout-track")
            .expect("staged change should be restored after timeout"),
    )
    .expect("restored staged change should serialize");
    assert_eq!(staged_after, staged_before);
    assert_eq!(
        server.context.mutation.changes.pending_ids(),
        vec!["backup-timeout-track".to_string()],
        "snapshot should be restored exactly once"
    );
    let parent_pid = fixture_pid(&fixture.parent_pid, "parent PID");
    let descendant_pid = fixture_pid(
        fixture
            .descendant_pid
            .as_deref()
            .expect("descendant fixture should record a PID"),
        "descendant PID",
    );
    wait_for_pid_exit(parent_pid, "write_xml backup parent").await;
    wait_for_pid_exit(descendant_pid, "write_xml backup descendant").await;
    assert!(
        !fixture.delayed_sentinel.exists(),
        "timeout fixture must not survive long enough to write its sentinel"
    );

    drop(timeout_override);
    write_executable_script(
        &fixture.script,
        "#!/bin/sh\necho 'fast backup succeeded'\nexit 0\n",
    );
    let retry_path = output_dir.path().join("retry.xml");
    let retry = tokio::time::timeout(
        Duration::from_secs(5),
        server.write_xml(Parameters(WriteXmlParams {
            skip_label_gate: Some(true),
            output_path: Some(retry_path.to_string_lossy().to_string()),
            playlists: None,
        })),
    )
    .await
    .expect("same-server retry should not deadlock")
    .expect("same-server retry should succeed");
    let payload = extract_json(&retry);
    assert_eq!(payload["changes_applied"], 1);
    assert_eq!(server.context.mutation.changes.pending_count(), 0);
    let xml = std::fs::read_to_string(&retry_path).expect("retry XML should be readable");
    assert!(xml.contains("Genre=\"Techno\""));
    assert!(xml.contains("Comments=\"restored after timeout\""));
    assert!(xml.contains("Rating=\"255\""));
}

#[tokio::test]
#[cfg(unix)]
async fn write_xml_backup_timeout_restores_state_and_releases_lock() {
    assert_write_xml_backup_failure_restores_state(WriteXmlBackupFailure::HungParent).await;
}

#[tokio::test]
#[cfg(unix)]
async fn write_xml_backup_early_exit_descendant_restores_state_and_releases_lock() {
    assert_write_xml_backup_failure_restores_state(WriteXmlBackupFailure::EarlyExitDescendant)
        .await;
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn write_xml_fails_closed_when_backup_script_missing_and_restores_changes() {
    let _env_guard = backup_script_env_lock()
        .lock()
        .expect("backup env mutex should not be poisoned");

    let db_conn = create_single_track_test_db("missing-backup-track", "/tmp/missing.flac");
    let store_dir = tempfile::tempdir().expect("temp store dir should create");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(
        store_path
            .to_str()
            .expect("temp store path should be UTF-8"),
    )
    .expect("temp internal store should open");
    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());

    server.context.mutation.changes.stage(vec![TrackChange {
        track_id: "missing-backup-track".to_string(),
        genre: Some("Techno".to_string()),
        ..Default::default()
    }]);

    let backup_dir = tempfile::tempdir().expect("temp backup dir should create");
    let missing_script = backup_dir.path().join("missing-backup.sh");
    let _backup_env = EnvVarGuard::set("REKLAWDBOX_BACKUP_SCRIPT", &missing_script);
    let output_dir = tempfile::tempdir().expect("temp output dir should create");
    let output_path = output_dir.path().join("must-not-exist.xml");

    let err = server
        .write_xml(Parameters(WriteXmlParams {
            skip_label_gate: Some(true),
            output_path: Some(output_path.to_string_lossy().to_string()),
            playlists: None,
        }))
        .await
        .expect_err("missing backup script must block XML export");

    let message = format!("{err:?}");
    assert!(message.contains(&missing_script.to_string_lossy().to_string()));
    assert!(!message.contains("REKORDBOX_DB_PATH="));
    assert!(!message.contains("environment"));
    assert!(!output_path.exists());
    assert_eq!(server.context.mutation.changes.pending_count(), 1);
}

pub(super) const WRITE_XML_TASK_TIMEOUT: Duration = Duration::from_secs(5);

pub(super) type WriteXmlTaskOutput = Result<CallToolResult, McpError>;

pub(super) struct WriteXmlTaskCleanup {
    handles: Vec<Option<tokio::task::JoinHandle<WriteXmlTaskOutput>>>,
}

impl WriteXmlTaskCleanup {
    pub(super) fn new() -> Self {
        Self {
            handles: Vec::new(),
        }
    }

    pub(super) fn push(&mut self, handle: tokio::task::JoinHandle<WriteXmlTaskOutput>) {
        self.handles.push(Some(handle));
    }

    pub(super) fn all_pending(&self) -> bool {
        self.handles
            .iter()
            .flatten()
            .all(|handle| !handle.is_finished())
    }

    pub(super) async fn join(
        &mut self,
        index: usize,
        phase: &str,
    ) -> Result<WriteXmlTaskOutput, String> {
        let mut handle = self
            .handles
            .get_mut(index)
            .and_then(Option::take)
            .ok_or_else(|| format!("{phase}: task handle is missing"))?;

        match tokio::time::timeout(WRITE_XML_TASK_TIMEOUT, &mut handle).await {
            Ok(Ok(output)) => Ok(output),
            Ok(Err(err)) => Err(format!("{phase}: task join failed: {err}")),
            Err(_) => {
                handle.abort();
                let cleanup = tokio::time::timeout(WRITE_XML_TASK_TIMEOUT, &mut handle).await;
                if cleanup.is_err() {
                    return Err(format!(
                        "{phase}: task timed out and abort cleanup did not finish within five seconds"
                    ));
                }
                Err(format!("{phase}: task did not finish within five seconds"))
            }
        }
    }

    pub(super) async fn abort(&mut self, index: usize, phase: &str) -> Result<(), String> {
        let mut handle = self
            .handles
            .get_mut(index)
            .and_then(Option::take)
            .ok_or_else(|| format!("{phase}: task handle is missing"))?;
        handle.abort();
        match tokio::time::timeout(WRITE_XML_TASK_TIMEOUT, &mut handle).await {
            Ok(Err(err)) if err.is_cancelled() => Ok(()),
            Ok(Err(err)) => Err(format!("{phase}: aborted task join failed: {err}")),
            Ok(Ok(_)) => Err(format!("{phase}: task completed before cancellation")),
            Err(_) => Err(format!(
                "{phase}: aborted task did not join within five seconds"
            )),
        }
    }

    pub(super) async fn abort_all(&mut self) -> Result<(), String> {
        for handle in self.handles.iter().flatten() {
            handle.abort();
        }

        for (index, slot) in self.handles.iter_mut().enumerate() {
            let Some(mut handle) = slot.take() else {
                continue;
            };
            if tokio::time::timeout(WRITE_XML_TASK_TIMEOUT, &mut handle)
                .await
                .is_err()
            {
                return Err(format!(
                    "task {index} did not join during cleanup within five seconds"
                ));
            }
        }
        Ok(())
    }
}

impl Drop for WriteXmlTaskCleanup {
    fn drop(&mut self) {
        for handle in self.handles.iter().flatten() {
            handle.abort();
        }
    }
}

pub(super) fn spawn_queued_write_xml(
    server: ReklawdboxServer,
    params: WriteXmlParams,
    queued: Arc<tokio::sync::Notify>,
) -> tokio::task::JoinHandle<WriteXmlTaskOutput> {
    tokio::spawn(async move {
        let mut request = Box::pin(server.write_xml(Parameters(params)));
        std::future::poll_fn(|cx| match request.as_mut().poll(cx) {
            std::task::Poll::Pending => std::task::Poll::Ready(()),
            std::task::Poll::Ready(_) => {
                panic!("write_xml completed instead of waiting for the held export lock")
            }
        })
        .await;
        queued.notify_one();
        request.await
    })
}

pub(super) async fn wait_for_queued_write_xml(
    queued: &tokio::sync::Notify,
    phase: &str,
) -> Result<(), String> {
    tokio::time::timeout(WRITE_XML_TASK_TIMEOUT, queued.notified())
        .await
        .map_err(|_| format!("{phase}: write_xml did not queue within five seconds"))
}
