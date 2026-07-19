use super::support::{
    EnvVarGuard, WRITE_XML_TASK_TIMEOUT, WriteXmlTaskCleanup, assert_path_remains_absent,
    assert_pre_op_backup_rejects_early_exit_descendant,
    assert_pre_op_backup_timeout_terminates_fixture, backup_archives, backup_script_env_lock,
    child_output_text, create_backup_archive_fixture, fixture_pid, kill_fixture_pid, pid_exists,
    run_embedded_backup_script, run_embedded_backup_script_with_temp_dir, spawn_queued_write_xml,
    tar_members, wait_for_nonempty_file, wait_for_pid_exit, wait_for_queued_write_xml,
    write_early_exit_backup_fixture, write_executable_script, write_hanging_backup_fixture,
};
use crate::mcp::metadata::{
    TrackChangeInput, UpdateTracksParams, WriteXmlParams, WriteXmlPlaylistInput,
};
use crate::mcp::server::ReklawdboxServer;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use rmcp::handler::server::wrapper::Parameters;

use crate::adapters::state as store;
use crate::domain::metadata::TrackChange;

use super::super::common::{
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

#[test]
#[cfg(unix)]
#[allow(clippy::await_holding_lock)]
fn effective_db_path_shared_with_backup_and_rejects_unsafe_paths() {
    use std::os::unix::fs::symlink;

    let _env_guard = backup_script_env_lock()
        .lock()
        .expect("backup env mutex should not be poisoned");
    let temp = tempfile::tempdir().expect("temp DB directory should create");
    let configured_dir = temp.path().join("Configured Library");
    std::fs::create_dir(&configured_dir).expect("configured DB directory should create");
    let configured = configured_dir.join("master.db");
    std::fs::write(&configured, []).expect("configured master.db should create");

    let alternate_dir = temp.path().join("Environment Library");
    std::fs::create_dir(&alternate_dir).expect("environment DB directory should create");
    let alternate = alternate_dir.join("master.db");
    std::fs::write(&alternate, []).expect("environment master.db should create");
    let _db_env = EnvVarGuard::set("REKORDBOX_DB_PATH", &alternate);

    let server = ReklawdboxServer::new(Some(configured.to_string_lossy().to_string()));
    let effective = server
        .effective_db_path()
        .expect("constructor override should resolve");
    assert_eq!(
        effective,
        configured
            .canonicalize()
            .expect("configured path should canonicalize")
    );
    assert_eq!(
        server
            .effective_db_path()
            .expect("cached effective path should resolve"),
        effective
    );

    let connection = server
        .rekordbox_conn()
        .expect("empty master.db should open through the production read-only path");
    assert!(
        connection
            .execute("CREATE TABLE forbidden_write (id INTEGER)", [])
            .is_err(),
        "production Rekordbox connection must remain read-only"
    );
    drop(connection);

    let misnamed = configured_dir.join("library.db");
    std::fs::write(&misnamed, []).expect("misnamed DB fixture should create");
    let misnamed_server = ReklawdboxServer::new(Some(misnamed.to_string_lossy().to_string()));
    let misnamed_error = misnamed_server
        .effective_db_path()
        .expect_err("misnamed configured DB must be rejected");
    assert!(misnamed_error.message.contains("must name master.db"));

    let symlink_dir = temp.path().join("Symlinked Library");
    std::fs::create_dir(&symlink_dir).expect("symlinked DB directory should create");
    let symlinked = symlink_dir.join("master.db");
    symlink(&configured, &symlinked).expect("symlink fixture should create");
    let symlink_server = ReklawdboxServer::new(Some(symlinked.to_string_lossy().to_string()));
    let symlink_error = symlink_server
        .effective_db_path()
        .expect_err("symlinked configured DB must be rejected");
    assert!(symlink_error.message.contains("symlinks are not supported"));
}

#[tokio::test]
#[cfg(unix)]
#[allow(clippy::await_holding_lock)]
async fn pre_op_backup_success_path_env_preserves_first_argument_and_parent_env() {
    let _env_guard = backup_script_env_lock()
        .lock()
        .expect("backup env mutex should not be poisoned");
    let temp = tempfile::tempdir().expect("temp backup fixture should create");
    let configured_dir = temp.path().join("Configured Library");
    std::fs::create_dir(&configured_dir).expect("configured directory should create");
    let configured = configured_dir.join("master.db");
    std::fs::write(&configured, []).expect("configured master.db should create");
    let canonical = configured
        .canonicalize()
        .expect("configured master.db should canonicalize");

    let parent_value = temp.path().join("parent-process-master.db");
    let _db_env = EnvVarGuard::set("REKORDBOX_DB_PATH", &parent_value);
    let marker = temp.path().join("custom-script-marker.txt");
    let script = temp.path().join("custom backup.sh");
    write_executable_script(
        &script,
        &format!(
            "#!/bin/sh\nprintf '%s\\n%s\\n' \"$1\" \"$REKORDBOX_DB_PATH\" > '{}'\n",
            marker.display()
        ),
    );
    let _script_env = EnvVarGuard::set("REKLAWDBOX_BACKUP_SCRIPT", &script);

    let status = crate::adapters::rekordbox::backup::run_pre_op_backup(&canonical)
        .await
        .expect("custom script zero exit should attest success");
    assert_eq!(
        status,
        crate::adapters::rekordbox::backup::BackupStatus::Success
    );
    let observed = std::fs::read_to_string(&marker).expect("custom script marker should exist");
    let mut lines = observed.lines();
    assert_eq!(lines.next(), Some("--pre-op"));
    assert_eq!(lines.next(), canonical.to_str());
    assert_eq!(lines.next(), None);
    assert_eq!(
        std::env::var_os("REKORDBOX_DB_PATH"),
        Some(parent_value.into())
    );
}

#[tokio::test]
#[cfg(unix)]
#[allow(clippy::await_holding_lock)]
async fn pre_op_backup_output_is_bounded() {
    let _env_guard = backup_script_env_lock()
        .lock()
        .expect("backup env mutex should not be poisoned");
    let temp = tempfile::tempdir().expect("temp backup fixture should create");
    let db_path = temp.path().join("master.db");
    std::fs::write(&db_path, []).expect("configured master.db should create");
    let script = temp.path().join("noisy-failure.sh");
    write_executable_script(
        &script,
        "#!/bin/sh\ni=0\nwhile [ \"$i\" -lt 9000 ]; do printf x >&2; i=$((i + 1)); done\nexit 17\n",
    );
    let _script_env = EnvVarGuard::set("REKLAWDBOX_BACKUP_SCRIPT", &script);

    let error = crate::adapters::rekordbox::backup::run_pre_op_backup(&db_path)
        .await
        .expect_err("nonzero custom script should fail");
    assert!(error.contains("exit status 17"));
    assert!(error.contains("[truncated]"));
    assert!(error.len() < 8_500, "failure output should remain bounded");
}

#[tokio::test]
#[cfg(unix)]
#[allow(clippy::await_holding_lock)]
async fn pre_op_backup_nonzero_exit_is_reported() {
    let _env_guard = backup_script_env_lock()
        .lock()
        .expect("backup env mutex should not be poisoned");
    let temp = tempfile::tempdir().expect("temp backup fixture should create");
    let db_path = temp.path().join("master.db");
    std::fs::write(&db_path, []).expect("configured master.db should create");
    let script = temp.path().join("nonzero.sh");
    write_executable_script(&script, "#!/bin/sh\necho 'nonzero backup' >&2\nexit 19\n");
    let _script_env = EnvVarGuard::set("REKLAWDBOX_BACKUP_SCRIPT", &script);

    let error = crate::adapters::rekordbox::backup::run_pre_op_backup(&db_path)
        .await
        .expect_err("nonzero custom script should fail");
    assert!(error.contains("exit status 19"));
    assert!(error.contains("nonzero backup"));
}

#[tokio::test]
#[cfg(unix)]
async fn pre_op_backup_timeout_reaps_direct_hung_child() {
    assert_pre_op_backup_timeout_terminates_fixture(false).await;
}

#[tokio::test]
#[cfg(unix)]
async fn pre_op_backup_timeout_reaps_descendant_holding_output_pipes() {
    assert_pre_op_backup_timeout_terminates_fixture(true).await;
}

#[tokio::test]
#[cfg(unix)]
async fn pre_op_backup_early_exit_reaps_descendant_holding_output_pipes() {
    assert_pre_op_backup_rejects_early_exit_descendant(false).await;
}

#[tokio::test]
#[cfg(unix)]
async fn pre_op_backup_early_exit_detects_descendant_that_closed_output_pipes() {
    assert_pre_op_backup_rejects_early_exit_descendant(true).await;
}

#[tokio::test]
#[cfg(unix)]
async fn pre_op_backup_cancellation_keeps_supervisor_cleanup_alive() {
    let temp = tempfile::tempdir().expect("cancelled backup fixture directory should create");
    let fixture = write_hanging_backup_fixture(temp.path(), true);
    let script = fixture.script.clone();
    let reader_activity = Arc::new(AtomicUsize::new(0));
    let task_activity = Arc::clone(&reader_activity);
    let caller = tokio::spawn(async move {
        crate::adapters::rekordbox::backup::execute_script_with_timeout_and_activity_for_test(
            &script,
            Duration::from_millis(250),
            task_activity,
        )
        .await
    });

    wait_for_nonempty_file(&fixture.ready, "cancelled backup ready marker").await;
    let parent_pid = fixture_pid(&fixture.parent_pid, "cancelled backup parent PID");
    let descendant_pid = fixture_pid(
        fixture
            .descendant_pid
            .as_deref()
            .expect("cancelled fixture should record a descendant PID"),
        "cancelled backup descendant PID",
    );
    tokio::time::timeout(Duration::from_secs(1), async {
        while reader_activity.load(Ordering::Acquire) != 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("cancelled backup output readers should start");

    caller.abort();
    assert!(
        caller
            .await
            .expect_err("cancelled caller task should not complete")
            .is_cancelled()
    );

    let cleanup = tokio::time::timeout(Duration::from_secs(5), async {
        while pid_exists(parent_pid)
            || pid_exists(descendant_pid)
            || reader_activity.load(Ordering::Acquire) != 0
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    if let Err(error) = cleanup {
        kill_fixture_pid(descendant_pid);
        kill_fixture_pid(parent_pid);
        panic!("cancelled backup supervisor did not quiesce: {error}");
    }
    assert_path_remains_absent(&fixture.delayed_sentinel).await;
}

#[test]
#[cfg(unix)]
fn embedded_backup_custom_db_path_uses_only_configured_directory() {
    let temp = tempfile::tempdir().expect("backup integration fixture should create");
    let home = temp.path().join("Isolated Home");
    let standard = home.join("Library/Pioneer/rekordbox");
    std::fs::create_dir_all(&standard).expect("fake standard library should create");
    std::fs::write(standard.join("master.db"), b"standard")
        .expect("fake standard master should create");
    std::fs::write(standard.join("networkAnalyze6.db"), b"standard-only")
        .expect("fake standard sentinel should create");

    let configured = temp.path().join("Configured Library With Spaces");
    std::fs::create_dir(&configured).expect("configured library should create");
    std::fs::write(configured.join("master.db"), b"configured")
        .expect("configured master should create");
    std::fs::write(configured.join("master.db-wal"), b"configured wal")
        .expect("configured WAL should create");
    std::fs::write(configured.join("product.db"), b"configured sentinel")
        .expect("configured sentinel should create");
    let db_path = configured.join("master.db");

    let db_output = run_embedded_backup_script(&["--db-only"], &home, Some(&db_path), None);
    assert!(
        db_output.status.success(),
        "configured DB backup should succeed: {}",
        child_output_text(&db_output)
    );
    let db_archives = backup_archives(&home, "db_");
    assert_eq!(db_archives.len(), 1);
    let db_members = tar_members(&db_archives[0]);
    assert!(db_members.contains(&"master.db".to_string()));
    assert!(db_members.contains(&"master.db-wal".to_string()));
    assert!(db_members.contains(&"product.db".to_string()));
    assert!(!db_members.contains(&"networkAnalyze6.db".to_string()));

    let pre_op_output = run_embedded_backup_script(&["--pre-op"], &home, Some(&db_path), None);
    assert!(
        pre_op_output.status.success(),
        "configured pre-op backup should succeed: {}",
        child_output_text(&pre_op_output)
    );
    let pre_op_archives = backup_archives(&home, "pre-op_");
    assert_eq!(pre_op_archives.len(), 1);
    assert_eq!(tar_members(&pre_op_archives[0]), db_members);
}

#[test]
#[cfg(unix)]
fn embedded_backup_mode_specific_path_rules() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().expect("backup path-rules fixture should create");
    let home = temp.path().join("Isolated Home");
    let standard = home.join("Library/Pioneer/rekordbox");
    std::fs::create_dir_all(&standard).expect("fake standard library should create");
    std::fs::write(standard.join("master.db"), b"standard")
        .expect("fake standard master should create");
    std::fs::write(standard.join("networkAnalyze6.db"), b"standard sentinel")
        .expect("fake standard sentinel should create");

    let missing = temp.path().join("Missing Library/master.db");
    let missing_output = run_embedded_backup_script(&["--db-only"], &home, Some(&missing), None);
    assert!(!missing_output.status.success());
    assert!(child_output_text(&missing_output).contains("not found"));
    assert!(backup_archives(&home, "db_").is_empty());

    let non_file_dir = temp.path().join("Directory Named master.db");
    std::fs::create_dir(&non_file_dir).expect("non-file configured path should create");
    let non_file_output =
        run_embedded_backup_script(&["--db-only"], &home, Some(&non_file_dir), None);
    assert!(!non_file_output.status.success());
    assert!(backup_archives(&home, "db_").is_empty());

    let misnamed = temp.path().join("library.db");
    std::fs::write(&misnamed, b"misnamed").expect("misnamed configured DB should create");
    let misnamed_output = run_embedded_backup_script(&["--db-only"], &home, Some(&misnamed), None);
    assert!(!misnamed_output.status.success());
    assert!(child_output_text(&misnamed_output).contains("must name master.db"));

    let real_dir = temp.path().join("Real Library");
    let symlink_dir = temp.path().join("Symlink Library");
    std::fs::create_dir(&real_dir).expect("real library should create");
    std::fs::create_dir(&symlink_dir).expect("symlink library should create");
    let real_db = real_dir.join("master.db");
    std::fs::write(&real_db, b"real").expect("real DB should create");
    let linked_db = symlink_dir.join("master.db");
    symlink(&real_db, &linked_db).expect("configured DB symlink should create");
    let symlink_output = run_embedded_backup_script(&["--db-only"], &home, Some(&linked_db), None);
    assert!(!symlink_output.status.success());
    assert!(child_output_text(&symlink_output).contains("symlinks are not supported"));
    assert!(backup_archives(&home, "db_").is_empty());

    for mode in [["--list"].as_slice(), ["--help"].as_slice()] {
        let output = run_embedded_backup_script(mode, &home, Some(&missing), None);
        assert!(
            output.status.success(),
            "non-producing mode should not validate a missing DB: {}",
            child_output_text(&output)
        );
    }

    let default_output = run_embedded_backup_script(&["--db-only"], &home, None, None);
    assert!(
        default_output.status.success(),
        "standard default source should remain supported: {}",
        child_output_text(&default_output)
    );
    let default_archives = backup_archives(&home, "db_");
    assert_eq!(default_archives.len(), 1);
    let default_members = tar_members(&default_archives[0]);
    assert!(default_members.contains(&"master.db".to_string()));
    assert!(default_members.contains(&"networkAnalyze6.db".to_string()));
}

#[test]
#[cfg(unix)]
fn backup_script_custom_path_restores_missing_db_safely() {
    let temp = tempfile::tempdir().expect("DB restore fixture should create");
    let archive_source = temp.path().join("Archive Source");
    std::fs::create_dir(&archive_source).expect("archive source should create");
    std::fs::write(archive_source.join("master.db"), b"restored master")
        .expect("restore master fixture should create");
    std::fs::write(archive_source.join("master.db-wal"), b"restored wal")
        .expect("restore WAL fixture should create");
    let archive = temp.path().join("db-restore-input.tar.gz");
    create_backup_archive_fixture(&archive, &archive_source, &["master.db", "master.db-wal"]);

    let empty_home = temp.path().join("Empty Target Home");
    let empty_standard = empty_home.join("Library/Pioneer/rekordbox");
    std::fs::create_dir_all(&empty_standard).expect("fake standard target should create");
    std::fs::write(empty_standard.join("master.db"), b"standard untouched")
        .expect("fake standard sentinel should create");
    let empty_target = temp.path().join("Empty Configured Target");
    std::fs::create_dir(&empty_target).expect("empty configured target should create");
    let archive_arg = archive.to_str().expect("temp archive path should be UTF-8");
    let empty_output = run_embedded_backup_script(
        &["--restore", archive_arg],
        &empty_home,
        Some(&empty_target.join("master.db")),
        Some("YES\n"),
    );
    assert!(
        empty_output.status.success(),
        "missing-master restore should succeed: {}",
        child_output_text(&empty_output)
    );
    assert!(
        child_output_text(&empty_output)
            .contains("No current database files to back up; continuing restore.")
    );
    assert_eq!(
        std::fs::read(empty_target.join("master.db")).expect("restored master should exist"),
        b"restored master"
    );
    assert_eq!(
        std::fs::read(empty_standard.join("master.db"))
            .expect("fake standard sentinel should remain"),
        b"standard untouched"
    );
    assert!(backup_archives(&empty_home, "pre-restore_").is_empty());

    let sidecar_home = temp.path().join("Sidecar Target Home");
    let sidecar_standard = sidecar_home.join("Library/Pioneer/rekordbox");
    std::fs::create_dir_all(&sidecar_standard).expect("second fake standard target should create");
    std::fs::write(
        sidecar_standard.join("master.db"),
        b"second standard untouched",
    )
    .expect("second fake standard sentinel should create");
    let sidecar_target = temp.path().join("Sidecar Configured Target");
    std::fs::create_dir(&sidecar_target).expect("sidecar configured target should create");
    std::fs::write(sidecar_target.join("master.db-wal"), b"current sidecar")
        .expect("current sidecar should create");
    let sidecar_output = run_embedded_backup_script(
        &["--restore", archive_arg],
        &sidecar_home,
        Some(&sidecar_target.join("master.db")),
        Some("YES\n"),
    );
    assert!(
        sidecar_output.status.success(),
        "sidecar safety backup and restore should succeed: {}",
        child_output_text(&sidecar_output)
    );
    assert!(
        !child_output_text(&sidecar_output)
            .contains("No current database files to back up; continuing restore.")
    );
    let safety_archives = backup_archives(&sidecar_home, "pre-restore_");
    assert_eq!(safety_archives.len(), 1);
    assert_eq!(tar_members(&safety_archives[0]), vec!["master.db-wal"]);
    assert_eq!(
        std::fs::read(sidecar_standard.join("master.db"))
            .expect("second fake standard sentinel should remain"),
        b"second standard untouched"
    );
}

#[test]
#[cfg(unix)]
fn backup_script_custom_path_full_round_trip_uses_canonical_root() {
    let temp = tempfile::tempdir().expect("full backup fixture should create");
    let home = temp.path().join("Isolated Home");
    let standard = home.join("Library/Pioneer/rekordbox");
    std::fs::create_dir_all(&standard).expect("fake standard library should create");
    std::fs::write(standard.join("master.db"), b"standard untouched")
        .expect("fake standard sentinel should create");

    let configured = temp.path().join("Target [Library] * With Different Name?");
    std::fs::create_dir(&configured).expect("configured full library should create");
    std::fs::create_dir(configured.join("sub directory"))
        .expect("configured nested directory should create");
    let many_files = configured.join("many files");
    std::fs::create_dir(&many_files).expect("many-files directory should create");
    std::fs::write(configured.join("master.db"), b"original master")
        .expect("configured master should create");
    std::fs::write(configured.join(".hidden"), b"hidden")
        .expect("configured hidden file should create");
    std::fs::write(configured.join("-leading"), b"leading")
        .expect("configured leading-dash file should create");
    std::fs::write(
        configured.join("sub directory/sentinel.txt"),
        b"nested original",
    )
    .expect("configured nested sentinel should create");
    for index in 0..320 {
        let name = format!("bulk-{index:03}-{}.txt", "x".repeat(160));
        std::fs::write(many_files.join(name), b"bulk restore fixture")
            .expect("bulk restore fixture should create");
    }
    let db_path = configured.join("master.db");

    let full_output = run_embedded_backup_script(&[], &home, Some(&db_path), None);
    assert!(
        full_output.status.success(),
        "configured full backup should succeed: {}",
        child_output_text(&full_output)
    );
    let full_archives = backup_archives(&home, "full_");
    assert_eq!(full_archives.len(), 1);
    let members = tar_members(&full_archives[0]);
    assert!(
        members.len() > 20,
        "full restore regression requires more than twenty archive members"
    );
    assert!(
        members
            .iter()
            .all(|member| member == "rekordbox" || member.starts_with("rekordbox/")),
        "full archive should have only the canonical root: {members:?}"
    );
    assert!(members.contains(&"rekordbox/master.db".to_string()));
    assert!(members.contains(&"rekordbox/.hidden".to_string()));
    assert!(members.contains(&"rekordbox/-leading".to_string()));
    assert!(members.contains(&"rekordbox/sub directory/sentinel.txt".to_string()));

    std::fs::write(configured.join("master.db"), b"mutated master")
        .expect("configured master should mutate");
    std::fs::remove_file(configured.join("sub directory/sentinel.txt"))
        .expect("original nested sentinel should remove");
    std::fs::write(configured.join("mutation-only.txt"), b"remove on restore")
        .expect("mutation-only sentinel should create");
    let archive_arg = full_archives[0]
        .to_str()
        .expect("temp archive path should be UTF-8");
    let restore_output = run_embedded_backup_script(
        &["--restore", archive_arg],
        &home,
        Some(&db_path),
        Some("YES\n"),
    );
    assert!(
        restore_output.status.success(),
        "configured full restore should succeed: {}",
        child_output_text(&restore_output)
    );
    assert_eq!(
        std::fs::read(configured.join("master.db")).expect("restored master should exist"),
        b"original master"
    );
    assert_eq!(
        std::fs::read(configured.join("sub directory/sentinel.txt"))
            .expect("restored nested sentinel should exist"),
        b"nested original"
    );
    assert!(!configured.join("mutation-only.txt").exists());
    assert_eq!(
        std::fs::read(standard.join("master.db")).expect("fake standard sentinel should remain"),
        b"standard untouched"
    );

    let safety_archives = backup_archives(&home, "full_pre-restore_");
    assert_eq!(safety_archives.len(), 1);
    let safety_members = tar_members(&safety_archives[0]);
    assert!(
        safety_members
            .iter()
            .all(|member| member == "rekordbox" || member.starts_with("rekordbox/")),
        "full safety archive should also use the canonical root: {safety_members:?}"
    );
}

#[test]
#[cfg(unix)]
fn backup_script_custom_path_nested_backup_directory_survives_full_restore() {
    let temp = tempfile::tempdir().expect("nested backup fixture should create");
    let home_and_library = temp.path().join("Nested Home And Library");
    let external_child_temp = temp.path().join("External Child Temp");
    std::fs::create_dir(&home_and_library).expect("nested configured library should create");
    let db_path = home_and_library.join("master.db");
    std::fs::write(&db_path, b"original nested master")
        .expect("nested configured master should create");
    std::fs::write(home_and_library.join("library-sentinel.txt"), b"original")
        .expect("nested library sentinel should create");

    let backup_output = run_embedded_backup_script_with_temp_dir(
        &[],
        &home_and_library,
        Some(&db_path),
        None,
        &external_child_temp,
    );
    assert!(
        backup_output.status.success(),
        "nested full backup should succeed: {}",
        child_output_text(&backup_output)
    );
    let input_archives = backup_archives(&home_and_library, "full_");
    assert_eq!(input_archives.len(), 1);
    let input_archive = input_archives[0].clone();
    assert!(input_archive.exists());

    std::fs::write(&db_path, b"mutated nested master")
        .expect("nested configured master should mutate");
    let archive_arg = input_archive
        .to_str()
        .expect("nested archive path should be UTF-8");
    let restore_output = run_embedded_backup_script_with_temp_dir(
        &["--restore", archive_arg],
        &home_and_library,
        Some(&db_path),
        Some("YES\n"),
        &external_child_temp,
    );
    assert!(
        restore_output.status.success(),
        "nested full restore should succeed: {}",
        child_output_text(&restore_output)
    );
    assert_eq!(
        std::fs::read(&db_path).expect("nested restored master should exist"),
        b"original nested master"
    );
    assert!(
        input_archive.exists(),
        "full restore must preserve its input archive when the backup directory is nested"
    );
    assert!(
        tar_members(&input_archive).contains(&"rekordbox/master.db".to_string()),
        "preserved input archive should remain readable"
    );
    let safety_archives = backup_archives(&home_and_library, "full_pre-restore_");
    assert_eq!(
        safety_archives.len(),
        1,
        "full restore must preserve its new safety archive when the backup directory is nested"
    );
    assert!(safety_archives[0].exists());
    assert!(
        tar_members(&safety_archives[0]).contains(&"rekordbox/master.db".to_string()),
        "preserved safety archive should remain readable"
    );
}

#[test]
#[cfg(unix)]
fn backup_script_custom_path_nested_backup_restore_failure_rolls_back_safely() {
    let temp = tempfile::tempdir().expect("nested rollback fixture should create");
    let home_and_library = temp.path().join("Nested Rollback Home And Library");
    let external_child_temp = temp.path().join("External Rollback Child Temp");
    let backup_dir = home_and_library.join("Music/rekordbox-backups");
    std::fs::create_dir_all(&backup_dir).expect("nested rollback backup directory should create");
    let db_path = home_and_library.join("master.db");
    std::fs::write(&db_path, b"current library must survive")
        .expect("nested rollback master should create");

    let crafted_source = temp.path().join("Crafted Conflicting Full Archive");
    let crafted_library = crafted_source.join("rekordbox");
    std::fs::create_dir_all(crafted_library.join("Music/rekordbox-backups"))
        .expect("crafted nested backup destination should create");
    std::fs::write(crafted_library.join("master.db"), b"must not install")
        .expect("crafted master should create");
    std::fs::write(
        crafted_library.join("Music/rekordbox-backups/conflict.txt"),
        b"must not replace preserved backups",
    )
    .expect("crafted backup conflict should create");
    let input_archive = backup_dir.join("full_conflicting-backup-dir.tar.gz");
    create_backup_archive_fixture(&input_archive, &crafted_source, &["rekordbox"]);

    let archive_arg = input_archive
        .to_str()
        .expect("nested rollback archive path should be UTF-8");
    let restore_output = run_embedded_backup_script_with_temp_dir(
        &["--restore", archive_arg],
        &home_and_library,
        Some(&db_path),
        Some("YES\n"),
        &external_child_temp,
    );
    assert!(
        !restore_output.status.success(),
        "conflicting restored backup directory must fail closed"
    );
    assert!(child_output_text(&restore_output).contains("attempting rollback"));
    assert_eq!(
        std::fs::read(&db_path).expect("current master should be rolled back"),
        b"current library must survive"
    );
    assert!(
        input_archive.exists(),
        "input archive must survive rollback"
    );
    assert!(
        tar_members(&input_archive).contains(&"rekordbox/master.db".to_string()),
        "rolled-back input archive should remain readable"
    );
    assert!(
        !backup_dir.join("conflict.txt").exists(),
        "failed restored backup contents must not replace preserved backups"
    );
    let safety_archives = backup_archives(&home_and_library, "full_pre-restore_");
    assert_eq!(safety_archives.len(), 1);
    assert!(safety_archives[0].exists());
    assert!(
        tar_members(&safety_archives[0]).contains(&"rekordbox/master.db".to_string()),
        "rolled-back safety archive should remain readable"
    );
}

#[tokio::test]
#[cfg(unix)]
#[allow(clippy::await_holding_lock)]
async fn backup_script_custom_path_rejects_symlink_before_custom_script() {
    use std::os::unix::fs::symlink;

    let _env_guard = backup_script_env_lock()
        .lock()
        .expect("backup env mutex should not be poisoned");
    let temp = tempfile::tempdir().expect("symlink export fixture should create");
    let real_dir = temp.path().join("Real Library");
    let symlink_dir = temp.path().join("Symlink Library");
    std::fs::create_dir(&real_dir).expect("real library should create");
    std::fs::create_dir(&symlink_dir).expect("symlink library should create");
    let real_db = real_dir.join("master.db");
    std::fs::write(&real_db, b"not opened").expect("real master should create");
    let linked_db = symlink_dir.join("master.db");
    symlink(&real_db, &linked_db).expect("configured DB symlink should create");

    let script = temp.path().join("custom-backup.sh");
    write_executable_script(
        &script,
        "#!/bin/sh\ntouch \"$(dirname \"$0\")/custom-script-ran\"\nexit 0\n",
    );
    let marker = temp.path().join("custom-script-ran");
    let _script_env = EnvVarGuard::set("REKLAWDBOX_BACKUP_SCRIPT", &script);
    let server = ReklawdboxServer::new(Some(linked_db.to_string_lossy().to_string()));
    server.context.mutation.changes.stage(vec![TrackChange {
        track_id: "symlink-track".to_string(),
        genre: Some("Techno".to_string()),
        ..Default::default()
    }]);
    let output_path = temp.path().join("must-not-exist.xml");

    let error = server
        .write_xml(Parameters(WriteXmlParams {
            skip_label_gate: Some(true),
            output_path: Some(output_path.to_string_lossy().to_string()),
            playlists: None,
        }))
        .await
        .expect_err("symlinked effective DB should fail before backup");
    assert!(error.message.contains("symlinks are not supported"));
    assert!(!marker.exists(), "custom backup script must not run");
    assert!(!output_path.exists(), "XML must not be created");
    assert_eq!(server.context.mutation.changes.pending_count(), 1);
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

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn pre_op_backup_missing_script_fails_closed() {
    let _env_guard = backup_script_env_lock()
        .lock()
        .expect("backup env mutex should not be poisoned");

    let backup_dir = tempfile::tempdir().expect("temp backup dir should create");
    let missing_script = backup_dir.path().join("missing-backup.sh");
    let db_path = backup_dir.path().join("master.db");
    std::fs::write(&db_path, b"test db").expect("temp master.db should create");
    let _backup_env = EnvVarGuard::set("REKLAWDBOX_BACKUP_SCRIPT", &missing_script);

    let error = crate::adapters::rekordbox::backup::run_pre_op_backup(&db_path)
        .await
        .expect_err("missing custom backup script must fail closed");
    assert!(error.contains(&missing_script.to_string_lossy().to_string()));
    assert!(!error.contains("REKORDBOX_DB_PATH="));
    assert!(!error.contains("environment"));
}
