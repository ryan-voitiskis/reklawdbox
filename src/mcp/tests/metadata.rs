use crate::mcp::enrichment::{
    set_test_bandcamp_lookup_override, set_test_musicbrainz_lookup_override,
};
use crate::mcp::metadata::{
    BackfillAlbumsParams, BackfillLabelsParams, BackfillYearsParams, TrackChangeInput,
    UpdateTracksParams, WriteXmlParams, WriteXmlPlaylistInput, handle_update_tracks,
};
use crate::mcp::server::ReklawdboxServer;
use std::future::Future;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use rmcp::ErrorData as McpError;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;

use crate::adapters::{rekordbox as db, state as store};
use crate::domain::classification::taxonomy as genre;
use crate::domain::metadata::ChangeManager;
use crate::domain::metadata::TrackChange;

use super::common::{
    call_tool_via_router, create_enrich_cache_writer_test_server,
    create_selector_pagination_test_db, create_server_with_connections,
    create_server_with_store_path, create_single_track_test_db, default_http_client_for_tests,
    extract_json, insert_test_track, make_test_track,
};

fn backup_script_env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

struct EnvVarGuard {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: &std::path::Path) -> Self {
        let previous = std::env::var_os(key);
        unsafe {
            std::env::set_var(key, value);
        }
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => unsafe { std::env::set_var(self.key, value) },
            None => unsafe { std::env::remove_var(self.key) },
        }
    }
}

#[cfg(unix)]
fn write_executable_script(path: &std::path::Path, contents: &str) {
    use std::os::unix::fs::PermissionsExt;

    std::fs::write(path, contents).expect("test script should be written");
    let mut permissions = std::fs::metadata(path)
        .expect("test script metadata should be readable")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).expect("test script should be executable");
}

#[cfg(unix)]
struct HangingBackupFixture {
    script: std::path::PathBuf,
    ready: std::path::PathBuf,
    parent_pid: std::path::PathBuf,
    descendant_pid: Option<std::path::PathBuf>,
    delayed_sentinel: std::path::PathBuf,
}

#[cfg(unix)]
fn shell_single_quote(path: &std::path::Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\"'\"'"))
}

#[cfg(unix)]
fn write_hanging_backup_fixture(
    directory: &std::path::Path,
    with_descendant: bool,
) -> HangingBackupFixture {
    let script = directory.join(if with_descendant {
        "descendant-hang.sh"
    } else {
        "direct-hang.sh"
    });
    let ready = directory.join("ready");
    let parent_pid = directory.join("parent.pid");
    let descendant_pid = with_descendant.then(|| directory.join("descendant.pid"));
    let delayed_sentinel = directory.join("survived-timeout");
    let block_fifo = directory.join("block.fifo");
    let ready_fifo = directory.join("ready.fifo");

    let contents = if let Some(descendant_pid) = descendant_pid.as_ref() {
        format!(
            "#!/bin/bash\n\
             set -eu\n\
             mkfifo {block_fifo} {ready_fifo}\n\
             exec 3<> {block_fifo}\n\
             exec 4<> {ready_fifo}\n\
             (\n\
               printf '%s\\n' \"$BASHPID\" > {descendant_pid}\n\
               printf 'descendant-ready\\n'\n\
               printf 'descendant-stderr-ready\\n' >&2\n\
               printf 'ready\\n' >&4\n\
               IFS= read -r -t 1 _ <&3 || true\n\
               printf 'descendant survived timeout\\n' > {delayed_sentinel}\n\
               while :; do IFS= read -r -t 60 _ <&3 || true; done\n\
             ) &\n\
             descendant=$!\n\
             printf '%s\\n' \"$$\" > {parent_pid}\n\
             IFS= read -r _ <&4\n\
             printf 'ready\\n' > {ready}\n\
             wait \"$descendant\"\n",
            block_fifo = shell_single_quote(&block_fifo),
            ready_fifo = shell_single_quote(&ready_fifo),
            descendant_pid = shell_single_quote(descendant_pid),
            delayed_sentinel = shell_single_quote(&delayed_sentinel),
            parent_pid = shell_single_quote(&parent_pid),
            ready = shell_single_quote(&ready),
        )
    } else {
        format!(
            "#!/bin/bash\n\
             set -eu\n\
             mkfifo {block_fifo}\n\
             exec 3<> {block_fifo}\n\
             printf '%s\\n' \"$$\" > {parent_pid}\n\
             printf 'parent-ready\\n'\n\
             printf 'parent-stderr-ready\\n' >&2\n\
             printf 'ready\\n' > {ready}\n\
             IFS= read -r -t 1 _ <&3 || true\n\
             printf 'parent survived timeout\\n' > {delayed_sentinel}\n\
             while :; do IFS= read -r -t 60 _ <&3 || true; done\n",
            block_fifo = shell_single_quote(&block_fifo),
            parent_pid = shell_single_quote(&parent_pid),
            ready = shell_single_quote(&ready),
            delayed_sentinel = shell_single_quote(&delayed_sentinel),
        )
    };
    write_executable_script(&script, &contents);
    HangingBackupFixture {
        script,
        ready,
        parent_pid,
        descendant_pid,
        delayed_sentinel,
    }
}

#[cfg(unix)]
async fn wait_for_nonempty_file(path: &std::path::Path, label: &str) -> String {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(contents) = std::fs::read_to_string(path)
                && !contents.trim().is_empty()
            {
                return contents;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {label}: {}", path.display()))
}

#[cfg(unix)]
fn fixture_pid(path: &std::path::Path, label: &str) -> i32 {
    std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("failed to read {label} {}: {error}", path.display()))
        .trim()
        .parse()
        .unwrap_or_else(|error| panic!("invalid {label} in {}: {error}", path.display()))
}

#[cfg(unix)]
fn pid_exists(pid: i32) -> bool {
    let result = unsafe { libc::kill(pid, 0) };
    result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(unix)]
fn kill_fixture_pid(pid: i32) {
    if pid_exists(pid) {
        unsafe {
            libc::kill(pid, libc::SIGKILL);
        }
    }
}

#[cfg(unix)]
async fn wait_for_pid_exit(pid: i32, label: &str) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while pid_exists(pid) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{label} PID {pid} remained alive"));
}

#[cfg(unix)]
async fn assert_path_remains_absent(path: &std::path::Path) {
    let deadline = tokio::time::Instant::now() + Duration::from_millis(1_200);
    while tokio::time::Instant::now() < deadline {
        assert!(
            !path.exists(),
            "delayed survival sentinel appeared: {}",
            path.display()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[cfg(unix)]
async fn assert_pre_op_backup_timeout_terminates_fixture(with_descendant: bool) {
    let temp = tempfile::tempdir().expect("hanging backup fixture directory should create");
    let fixture = write_hanging_backup_fixture(temp.path(), with_descendant);
    let script = fixture.script.clone();
    let mut backup_task = tokio::spawn(async move {
        crate::adapters::rekordbox::backup::execute_script_with_timeout_for_test(
            &script,
            Duration::from_millis(250),
        )
        .await
    });

    wait_for_nonempty_file(&fixture.ready, "backup ready marker").await;
    let parent_pid = fixture_pid(&fixture.parent_pid, "parent PID");
    let descendant_pid = fixture
        .descendant_pid
        .as_deref()
        .map(|path| fixture_pid(path, "descendant PID"));

    let result = match tokio::time::timeout(Duration::from_secs(5), &mut backup_task).await {
        Ok(joined) => joined.expect("backup timeout task should join"),
        Err(_) => {
            if let Some(pid) = descendant_pid {
                kill_fixture_pid(pid);
            }
            kill_fixture_pid(parent_pid);
            let _ = tokio::time::timeout(Duration::from_secs(5), &mut backup_task).await;
            panic!("pre-operation backup ignored its injected 250ms timeout");
        }
    };
    let error = result.expect_err("hanging pre-operation backup should time out");
    assert!(
        error.starts_with("pre-operation backup timed out after"),
        "unexpected timeout error: {error}"
    );

    wait_for_pid_exit(parent_pid, "backup parent").await;
    if let Some(pid) = descendant_pid {
        wait_for_pid_exit(pid, "backup descendant").await;
    }
    assert_path_remains_absent(&fixture.delayed_sentinel).await;
}

#[cfg(unix)]
fn run_embedded_backup_script(
    args: &[&str],
    home: &std::path::Path,
    db_path: Option<&std::path::Path>,
    stdin: Option<&str>,
) -> std::process::Output {
    run_embedded_backup_script_with_temp_dir(args, home, db_path, stdin, &home.join("tmp"))
}

#[cfg(unix)]
fn run_embedded_backup_script_with_temp_dir(
    args: &[&str],
    home: &std::path::Path,
    db_path: Option<&std::path::Path>,
    stdin: Option<&str>,
    temp_dir: &std::path::Path,
) -> std::process::Output {
    use std::io::Write as _;
    use std::os::unix::process::CommandExt as _;
    use std::process::{Command, Stdio};
    use std::time::Instant;

    let (script, _script_dir) =
        crate::adapters::rekordbox::backup::write_embedded_script_for_test()
            .expect("embedded backup script should be materialized");
    let fake_bin = home.join("test-bin");
    std::fs::create_dir_all(&fake_bin).expect("fake binary directory should create");
    write_executable_script(&fake_bin.join("pgrep"), "#!/bin/sh\nexit 1\n");
    std::fs::create_dir_all(temp_dir).expect("child temp directory should create");

    let mut command = Command::new("/bin/bash");
    command
        .arg(&script)
        .args(args)
        .env_clear()
        .env("HOME", home)
        .env("TMPDIR", temp_dir)
        .env("PATH", format!("{}:/usr/bin:/bin", fake_bin.display()))
        .env("LANG", "C")
        .process_group(0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(path) = db_path {
        command.env("REKORDBOX_DB_PATH", path);
    }

    let mut child = command
        .spawn()
        .expect("embedded backup child should launch");
    if let Some(input) = stdin {
        child
            .stdin
            .take()
            .expect("backup child stdin should be piped")
            .write_all(input.as_bytes())
            .expect("backup child input should be written");
    } else {
        drop(child.stdin.take());
    }

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match child
            .try_wait()
            .expect("backup child status should be readable")
        {
            Some(_) => break,
            None if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            None => {
                let process_group = -(child.id() as i32);
                unsafe {
                    libc::kill(process_group, libc::SIGKILL);
                }
                let _ = child.wait();
                panic!("embedded backup child exceeded the 10-second test timeout");
            }
        }
    }

    child
        .wait_with_output()
        .expect("backup child output should be collected")
}

#[cfg(unix)]
fn child_output_text(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[cfg(unix)]
fn backup_archives(home: &std::path::Path, prefix: &str) -> Vec<std::path::PathBuf> {
    let backup_dir = home.join("Music/rekordbox-backups");
    let mut archives = if backup_dir.is_dir() {
        std::fs::read_dir(&backup_dir)
            .expect("backup directory should be readable")
            .map(|entry| {
                entry
                    .expect("backup directory entry should be readable")
                    .path()
            })
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(prefix) && name.ends_with(".tar.gz"))
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    archives.sort();
    archives
}

#[cfg(unix)]
fn tar_members(archive: &std::path::Path) -> Vec<String> {
    let output = std::process::Command::new("tar")
        .args(["-tzf"])
        .arg(archive)
        .output()
        .expect("tar member listing should launch");
    assert!(
        output.status.success(),
        "tar member listing should succeed: {}",
        child_output_text(&output)
    );
    String::from_utf8(output.stdout)
        .expect("tar member names should be UTF-8 test paths")
        .lines()
        .map(str::to_owned)
        .collect()
}

#[cfg(unix)]
fn create_backup_archive_fixture(
    archive: &std::path::Path,
    source: &std::path::Path,
    members: &[&str],
) {
    let output = std::process::Command::new("tar")
        .args(["-czf"])
        .arg(archive)
        .arg("-C")
        .arg(source)
        .arg("--")
        .args(members)
        .output()
        .expect("DB backup fixture creation should launch");
    assert!(
        output.status.success(),
        "DB backup fixture creation should succeed: {}",
        child_output_text(&output)
    );
}

const WRITE_XML_TASK_TIMEOUT: Duration = Duration::from_secs(5);

type WriteXmlTaskOutput = Result<CallToolResult, McpError>;

struct WriteXmlTaskCleanup {
    handles: Vec<Option<tokio::task::JoinHandle<WriteXmlTaskOutput>>>,
}

impl WriteXmlTaskCleanup {
    fn new() -> Self {
        Self {
            handles: Vec::new(),
        }
    }

    fn push(&mut self, handle: tokio::task::JoinHandle<WriteXmlTaskOutput>) {
        self.handles.push(Some(handle));
    }

    fn all_pending(&self) -> bool {
        self.handles
            .iter()
            .flatten()
            .all(|handle| !handle.is_finished())
    }

    async fn join(&mut self, index: usize, phase: &str) -> Result<WriteXmlTaskOutput, String> {
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

    async fn abort(&mut self, index: usize, phase: &str) -> Result<(), String> {
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

    async fn abort_all(&mut self) -> Result<(), String> {
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

fn spawn_queued_write_xml(
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

async fn wait_for_queued_write_xml(
    queued: &tokio::sync::Notify,
    phase: &str,
) -> Result<(), String> {
    tokio::time::timeout(WRITE_XML_TASK_TIMEOUT, queued.notified())
        .await
        .map_err(|_| format!("{phase}: write_xml did not queue within five seconds"))
}

#[tokio::test]
async fn backfill_labels_conflict_page_later_dry_run_does_not_repeat_staging() {
    let db_conn = create_selector_pagination_test_db();
    db_conn
        .execute("UPDATE djmdContent SET LabelID = NULL WHERE ID = 't3'", [])
        .expect("unlabeled staging fixture should update");
    let tracks = db::get_tracks_by_ids(
        &db_conn,
        &["t1".to_string(), "t2".to_string(), "t3".to_string()],
    )
    .expect("label fixture tracks should load");
    let (server, _store_dir, _store_path) = create_enrich_cache_writer_test_server(db_conn);
    {
        let store_conn = server.cache_store_conn().expect("test store should open");
        for track in &tracks {
            let label = if track.id == "t3" {
                "Filled Label"
            } else {
                "Conflicting Label"
            };
            let response = serde_json::json!({"label": label}).to_string();
            store::set_enrichment(
                &store_conn,
                "discogs",
                &crate::domain::metadata::normalize_for_matching(&track.artist),
                &crate::domain::metadata::normalize_for_matching(&track.title),
                Some(&crate::domain::metadata::normalize_for_matching(
                    &track.album,
                )),
                Some("exact"),
                Some(&response),
            )
            .expect("label cache fixture should write");
        }
    }

    let first = server
        .backfill_labels(Parameters(BackfillLabelsParams {
            dry_run: Some(false),
            auto_enrich: Some(false),
            max_conflicts: Some(1),
            conflict_offset: Some(0),
        }))
        .await
        .expect("mutating label pass should succeed");
    let first_payload = extract_json(&first);
    assert_eq!(first_payload["staged"], 1);
    assert_eq!(first_payload["conflict_page"]["returned"], 1);
    assert_eq!(first_payload["conflict_page"]["next_offset"], 1);
    assert_eq!(first_payload["conflicts_truncated"], true);
    let pending_after_first = server.context.mutation.changes.pending_ids();
    assert_eq!(pending_after_first, vec!["t3".to_string()]);

    let second = server
        .backfill_labels(Parameters(BackfillLabelsParams {
            dry_run: Some(true),
            auto_enrich: Some(false),
            max_conflicts: Some(1),
            conflict_offset: Some(1),
        }))
        .await
        .expect("later dry-run conflict page should succeed");
    let second_payload = extract_json(&second);
    assert_eq!(second_payload["staged"], 0);
    assert_eq!(second_payload["conflict_page"]["offset"], 1);
    assert_eq!(second_payload["conflict_page"]["returned"], 1);
    assert_eq!(
        second_payload["conflict_page"]["next_offset"],
        serde_json::Value::Null
    );
    assert!(second_payload.get("conflicts_truncated").is_none());
    assert_eq!(
        server.context.mutation.changes.pending_ids(),
        pending_after_first
    );
}

#[tokio::test]
async fn backfill_albums_backfill_cache_persistence_reports_acknowledged_match() {
    let db_conn = create_single_track_test_db(
        "album-cache-persistence-match",
        "/tmp/album-cache-persistence-match.flac",
    );
    db_conn
        .execute_batch(
            "UPDATE djmdArtist SET Name = 'Album Persistence Artist' WHERE ID = 'a1';
             UPDATE djmdContent
             SET Title = 'Album Persistence Title', AlbumID = NULL
             WHERE ID = 'album-cache-persistence-match';",
        )
        .expect("album persistence fixture should need enrichment");

    let store_dir = tempfile::tempdir().expect("album persistence store should create");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_path_string = store_path.to_string_lossy().to_string();
    let store_conn = store::open(&store_path_string).expect("album persistence store should open");
    let http = reqwest::Client::builder()
        .proxy(
            reqwest::Proxy::all("http://127.0.0.1:9").expect("closed local proxy URL should parse"),
        )
        .timeout(Duration::from_millis(250))
        .build()
        .expect("album persistence HTTP client should build");
    let server =
        create_server_with_store_path(db_conn, store_conn, http, Some(store_path_string.clone()));
    set_test_bandcamp_lookup_override(
        "Album Persistence Artist",
        "Album Persistence Title",
        Ok(Some(crate::adapters::providers::bandcamp::BandcampResult {
            track_title: "Album Persistence Title".into(),
            artist_name: "Album Persistence Artist".into(),
            release_date: Some("2024-01-01".into()),
            label: Some("Album Persistence Label".into()),
            tags: vec![],
            album: Some("Durable Album".into()),
            cover_image: None,
            bandcamp_url: "https://example.bandcamp.com/track/durable".into(),
            score: 100,
        })),
    );

    let result = tokio::time::timeout(
        Duration::from_secs(5),
        server.backfill_albums(Parameters(BackfillAlbumsParams {
            dry_run: Some(false),
            auto_enrich: Some(true),
        })),
    )
    .await
    .expect("album persistence handler should finish within five seconds")
    .expect("album persistence handler should return partial success");
    let payload = extract_json(&result);
    assert_eq!(payload["auto_enrichment"]["requested"], 1);
    assert_eq!(payload["auto_enrichment"]["matched"], 1);
    assert_eq!(payload["auto_enrichment"]["cache_writes_succeeded"], 1);
    assert_eq!(payload["auto_enriched"], 1);
}

#[tokio::test]
async fn backfill_albums_backfill_cache_persistence_distinguishes_no_match_from_lookup_failure() {
    let db_conn = create_single_track_test_db("album-outcome-none", "/tmp/album-outcome-none.flac");
    insert_test_track(
        &db_conn,
        "album-outcome-error",
        "Album Outcome Error",
        "g1",
        "/tmp/album-outcome-error.flac",
    );
    db_conn
        .execute_batch(
            "UPDATE djmdArtist SET Name = 'Album Outcome Artist' WHERE ID = 'a1';
             UPDATE djmdContent
             SET Title = 'Album Outcome None', AlbumID = NULL
             WHERE ID = 'album-outcome-none';
             UPDATE djmdContent SET AlbumID = NULL WHERE ID = 'album-outcome-error';",
        )
        .expect("album outcome fixture should need enrichment");
    let (server, _store_dir, _store_path) = create_enrich_cache_writer_test_server(db_conn);
    set_test_bandcamp_lookup_override("Album Outcome Artist", "Album Outcome None", Ok(None));
    set_test_bandcamp_lookup_override(
        "Album Outcome Artist",
        "Album Outcome Error",
        Err("synthetic Bandcamp album failure".into()),
    );

    let result = tokio::time::timeout(
        Duration::from_secs(5),
        server.backfill_albums(Parameters(BackfillAlbumsParams {
            dry_run: Some(false),
            auto_enrich: Some(true),
        })),
    )
    .await
    .expect("album outcome handler should finish within five seconds")
    .expect("album outcome handler should return structured partial success");
    let payload = extract_json(&result);
    let report = &payload["auto_enrichment"];
    assert_eq!(payload["auto_enriched"], 0);
    assert_eq!(report["requested"], 2);
    assert_eq!(report["matched"], 0);
    assert_eq!(report["no_match"], 1);
    assert_eq!(report["lookup_failed"], 1);
    assert_eq!(report["cache_writes_succeeded"], 1);
    assert_eq!(report["cache_writes_failed"], 0);
    assert_eq!(report["operation_failed"], true);
    assert_eq!(report["by_provider"]["bandcamp"]["requested"], 2);
    assert_eq!(report["by_provider"]["bandcamp"]["no_match"], 1);
    assert_eq!(report["by_provider"]["bandcamp"]["lookup_failed"], 1);
    assert_eq!(report["failures"].as_array().map(Vec::len), Some(1));
    assert_eq!(report["failures"][0]["provider"], "bandcamp");
    assert_eq!(
        report["failures"][0]["normalized_title"],
        "album outcome error"
    );
    assert_eq!(report["failures"][0]["kind"], "lookup_failed");
    assert_eq!(payload["staged"], 0);

    let connection = server
        .cache_store_conn()
        .expect("album outcome store should open");
    let norm_artist = crate::domain::metadata::normalize_for_matching("Album Outcome Artist");
    let none_title = crate::domain::metadata::normalize_for_matching("Album Outcome None");
    let error_title = crate::domain::metadata::normalize_for_matching("Album Outcome Error");
    let no_match = store::get_enrichment(
        &connection,
        "bandcamp",
        &norm_artist,
        &none_title,
        None,
        false,
    )
    .expect("album no-match cache should read")
    .expect("album no-match should persist");
    assert_eq!(no_match.match_quality.as_deref(), Some("none"));
    assert!(no_match.response_json.is_none());
    assert!(
        store::get_enrichment(
            &connection,
            "bandcamp",
            &norm_artist,
            &error_title,
            None,
            false,
        )
        .expect("album lookup-failure cache should read")
        .is_none(),
        "album lookup failures must remain retryable"
    );
}

#[tokio::test]
async fn backfill_years_backfill_cache_persistence_reports_acknowledged_no_matches() {
    let db_conn = create_single_track_test_db(
        "year-cache-persistence-none",
        "/tmp/year-cache-persistence-none.flac",
    );
    db_conn
        .execute_batch(
            "UPDATE djmdArtist SET Name = 'Year Persistence Artist' WHERE ID = 'a1';
             UPDATE djmdContent
             SET Title = 'Year Persistence Title', ReleaseYear = 0
             WHERE ID = 'year-cache-persistence-none';",
        )
        .expect("year persistence fixture should need enrichment");
    let (server, _store_dir, _store_path) = create_enrich_cache_writer_test_server(db_conn);
    set_test_bandcamp_lookup_override(
        "Year Persistence Artist",
        "Year Persistence Title",
        Ok(None),
    );
    set_test_musicbrainz_lookup_override(
        "Year Persistence Artist",
        "Year Persistence Title",
        Ok(None),
    );

    let result = tokio::time::timeout(
        Duration::from_secs(5),
        server.backfill_years(Parameters(BackfillYearsParams {
            dry_run: Some(true),
            auto_enrich: Some(true),
        })),
    )
    .await
    .expect("year persistence handler should finish within five seconds")
    .expect("year persistence handler should return partial success");
    let payload = extract_json(&result);
    assert_eq!(payload["auto_enrichment"]["requested"], 2);
    assert_eq!(payload["auto_enrichment"]["no_match"], 2);
    assert_eq!(payload["auto_enrichment"]["cache_writes_succeeded"], 2);
    assert_eq!(payload["auto_enriched"], 0);
}

#[tokio::test]
async fn backfill_years_backfill_cache_persistence_rescans_both_positive_provider_rows() {
    let db_conn = create_single_track_test_db("year-positive-both", "/tmp/year-positive-both.flac");
    db_conn
        .execute_batch(
            "UPDATE djmdArtist SET Name = 'Year Positive Artist' WHERE ID = 'a1';
             UPDATE djmdContent
             SET Title = 'Year Positive Title', ReleaseYear = 0
             WHERE ID = 'year-positive-both';",
        )
        .expect("year positive fixture should need enrichment");
    let (server, _store_dir, _store_path) = create_enrich_cache_writer_test_server(db_conn);
    set_test_musicbrainz_lookup_override(
        "Year Positive Artist",
        "Year Positive Title",
        Ok(Some(
            crate::adapters::providers::musicbrainz::MusicBrainzResult {
                recording_title: "Year Positive Title".into(),
                artist: "Year Positive Artist".into(),
                first_release_date: Some("2022-03-04".into()),
                label: Some("Year Positive MusicBrainz".into()),
                score: 100,
            },
        )),
    );
    set_test_bandcamp_lookup_override(
        "Year Positive Artist",
        "Year Positive Title",
        Ok(Some(crate::adapters::providers::bandcamp::BandcampResult {
            track_title: "Year Positive Title".into(),
            artist_name: "Year Positive Artist".into(),
            release_date: Some("2023-05-06".into()),
            label: Some("Year Positive Bandcamp".into()),
            tags: vec![],
            album: Some("Year Positive Album".into()),
            cover_image: None,
            bandcamp_url: "https://example.bandcamp.com/track/year-positive".into(),
            score: 100,
        })),
    );

    let result = tokio::time::timeout(
        Duration::from_secs(5),
        server.backfill_years(Parameters(BackfillYearsParams {
            dry_run: Some(false),
            auto_enrich: Some(true),
        })),
    )
    .await
    .expect("year positive handler should finish within five seconds")
    .expect("year positive handler should succeed");
    let payload = extract_json(&result);
    let report = &payload["auto_enrichment"];
    assert_eq!(payload["auto_enriched"], 2);
    assert_eq!(report["requested"], 2);
    assert_eq!(report["matched"], 2);
    assert_eq!(report["no_match"], 0);
    assert_eq!(report["lookup_failed"], 0);
    assert_eq!(report["cache_writes_succeeded"], 2);
    assert_eq!(report["cache_writes_failed"], 0);
    assert_eq!(report["operation_failed"], false);
    for provider in ["bandcamp", "musicbrainz"] {
        assert_eq!(report["by_provider"][provider]["requested"], 1);
        assert_eq!(report["by_provider"][provider]["matched"], 1);
        assert_eq!(report["by_provider"][provider]["cache_writes_succeeded"], 1);
    }
    assert_eq!(payload["summary"]["filled_by_source"]["musicbrainz"], 1);
    assert_eq!(payload["summary"]["filled_by_source"]["bandcamp"], 0);
    assert_eq!(payload["staged"], 1);
    assert_eq!(
        server
            .context
            .mutation
            .changes
            .get("year-positive-both")
            .expect("positive year should be staged")
            .year,
        Some(2022),
        "MusicBrainz remains ahead of Bandcamp in the year cascade"
    );

    let connection = server
        .cache_store_conn()
        .expect("year positive store should open");
    let norm_artist = crate::domain::metadata::normalize_for_matching("Year Positive Artist");
    let norm_title = crate::domain::metadata::normalize_for_matching("Year Positive Title");
    for (provider, date_field, expected_date) in [
        ("bandcamp", "release_date", "2023-05-06"),
        ("musicbrainz", "first_release_date", "2022-03-04"),
    ] {
        let cached = store::get_enrichment(
            &connection,
            provider,
            &norm_artist,
            &norm_title,
            None,
            false,
        )
        .expect("year positive cache should read")
        .expect("each positive provider row should persist");
        assert_eq!(cached.match_quality.as_deref(), Some("exact"));
        let response: serde_json::Value = serde_json::from_str(
            cached
                .response_json
                .as_deref()
                .expect("positive year row should retain provider JSON"),
        )
        .expect("positive year row JSON should parse");
        assert_eq!(response[date_field], expected_date);
    }
}

#[tokio::test]
async fn backfill_years_backfill_cache_persistence_keeps_lookup_failure_retryable() {
    let db_conn = create_single_track_test_db("year-outcome-error", "/tmp/year-outcome-error.flac");
    db_conn
        .execute_batch(
            "UPDATE djmdArtist SET Name = 'Year Outcome Artist' WHERE ID = 'a1';
             UPDATE djmdContent
             SET Title = 'Year Outcome Title', ReleaseYear = 0
             WHERE ID = 'year-outcome-error';",
        )
        .expect("year outcome fixture should need enrichment");
    let (server, _store_dir, _store_path) = create_enrich_cache_writer_test_server(db_conn);
    set_test_musicbrainz_lookup_override(
        "Year Outcome Artist",
        "Year Outcome Title",
        Err("synthetic MusicBrainz year failure".into()),
    );
    set_test_bandcamp_lookup_override("Year Outcome Artist", "Year Outcome Title", Ok(None));

    let result = tokio::time::timeout(
        Duration::from_secs(5),
        server.backfill_years(Parameters(BackfillYearsParams {
            dry_run: Some(false),
            auto_enrich: Some(true),
        })),
    )
    .await
    .expect("year outcome handler should finish within five seconds")
    .expect("year outcome handler should return structured partial success");
    let payload = extract_json(&result);
    let report = &payload["auto_enrichment"];
    assert_eq!(payload["auto_enriched"], 0);
    assert_eq!(report["requested"], 2);
    assert_eq!(report["matched"], 0);
    assert_eq!(report["no_match"], 1);
    assert_eq!(report["lookup_failed"], 1);
    assert_eq!(report["cache_writes_succeeded"], 1);
    assert_eq!(report["cache_writes_failed"], 0);
    assert_eq!(report["operation_failed"], true);
    assert_eq!(report["by_provider"]["musicbrainz"]["lookup_failed"], 1);
    assert_eq!(report["by_provider"]["bandcamp"]["no_match"], 1);
    assert_eq!(report["failures"].as_array().map(Vec::len), Some(1));
    assert_eq!(report["failures"][0]["provider"], "musicbrainz");
    assert_eq!(
        report["failures"][0]["normalized_title"],
        "year outcome title"
    );
    assert_eq!(report["failures"][0]["kind"], "lookup_failed");
    assert_eq!(payload["staged"], 0);

    let connection = server
        .cache_store_conn()
        .expect("year outcome store should open");
    let norm_artist = crate::domain::metadata::normalize_for_matching("Year Outcome Artist");
    let norm_title = crate::domain::metadata::normalize_for_matching("Year Outcome Title");
    assert!(
        store::get_enrichment(
            &connection,
            "musicbrainz",
            &norm_artist,
            &norm_title,
            None,
            false,
        )
        .expect("MusicBrainz year cache should read")
        .is_none(),
        "year lookup failures must remain retryable"
    );
    let no_match = store::get_enrichment(
        &connection,
        "bandcamp",
        &norm_artist,
        &norm_title,
        None,
        false,
    )
    .expect("Bandcamp year cache should read")
    .expect("year no-match should persist");
    assert_eq!(no_match.match_quality.as_deref(), Some("none"));
    assert!(no_match.response_json.is_none());
}

#[tokio::test]
async fn backfill_labels_backfill_cache_persistence_auto_enriches_both_providers_and_preserves_precedence()
 {
    let db_conn = create_single_track_test_db(
        "labels-auto-enrich-match",
        "/music/labels-auto-enrich-match.flac",
    );
    db_conn
        .execute_batch(
            "UPDATE djmdArtist SET Name = 'Auto Match Artist' WHERE ID = 'a1';
             UPDATE djmdContent
             SET Title = 'Auto Match Title', LabelID = NULL
             WHERE ID = 'labels-auto-enrich-match';",
        )
        .expect("label fixture should become unlabeled");
    let (server, _store_dir, _store_path) = create_enrich_cache_writer_test_server(db_conn);

    set_test_musicbrainz_lookup_override(
        "Auto Match Artist",
        "Auto Match Title",
        Ok(Some(
            crate::adapters::providers::musicbrainz::MusicBrainzResult {
                recording_title: "Auto Match Title".into(),
                artist: "Auto Match Artist".into(),
                first_release_date: Some("2025-01-01".into()),
                label: Some("MusicBrainz Label".into()),
                score: 100,
            },
        )),
    );
    set_test_bandcamp_lookup_override(
        "Auto Match Artist",
        "Auto Match Title",
        Ok(Some(crate::adapters::providers::bandcamp::BandcampResult {
            track_title: "Auto Match Title".into(),
            artist_name: "Auto Match Artist".into(),
            release_date: Some("2025-01-02".into()),
            label: Some("Bandcamp Label".into()),
            tags: vec![],
            album: Some("Encoded Paths".into()),
            cover_image: None,
            bandcamp_url: "https://example.bandcamp.com/track/senorita".into(),
            score: 100,
        })),
    );

    let result = tokio::time::timeout(
        Duration::from_secs(5),
        server.backfill_labels(Parameters(BackfillLabelsParams {
            dry_run: Some(false),
            auto_enrich: Some(true),
            max_conflicts: None,
            conflict_offset: None,
        })),
    )
    .await
    .expect("dual-provider label handler should finish within five seconds")
    .expect("dual-provider label hydration should succeed");
    let payload = extract_json(&result);

    assert_eq!(payload["auto_enriched"], 2);
    assert_eq!(payload["auto_enriched_by_provider"]["musicbrainz"], 1);
    assert_eq!(payload["auto_enriched_by_provider"]["bandcamp"], 1);
    assert_eq!(payload["auto_enrichment"]["requested"], 2);
    assert_eq!(payload["auto_enrichment"]["matched"], 2);
    assert_eq!(payload["auto_enrichment"]["cache_writes_succeeded"], 2);
    assert_eq!(payload["auto_enrichment"]["cache_writes_failed"], 0);
    assert_eq!(payload["auto_enrichment"]["operation_failed"], false);
    assert_eq!(
        payload["auto_enrichment"]["failures"],
        serde_json::json!([])
    );
    for provider in ["musicbrainz", "bandcamp"] {
        assert_eq!(
            payload["auto_enrichment"]["by_provider"][provider]["requested"],
            1
        );
        assert_eq!(
            payload["auto_enrichment"]["by_provider"][provider]["matched"],
            1
        );
        assert_eq!(
            payload["auto_enrichment"]["by_provider"][provider]["cache_writes_succeeded"],
            1
        );
    }
    assert_eq!(payload["staged"], 1);
    let pending = server
        .context
        .mutation
        .changes
        .get("labels-auto-enrich-match")
        .expect("label fill should be staged");
    assert_eq!(pending.label.as_deref(), Some("MusicBrainz Label"));

    let store_conn = server.cache_store_conn().expect("test store should open");
    let norm_artist = crate::domain::metadata::normalize_for_matching("Auto Match Artist");
    let norm_title = crate::domain::metadata::normalize_for_matching("Auto Match Title");
    for (provider, expected_label) in [
        ("musicbrainz", "MusicBrainz Label"),
        ("bandcamp", "Bandcamp Label"),
    ] {
        let cached = store::get_enrichment(
            &store_conn,
            provider,
            &norm_artist,
            &norm_title,
            None,
            false,
        )
        .expect("provider cache should be readable")
        .expect("provider result should be cached");
        assert_eq!(cached.match_quality.as_deref(), Some("exact"));
        let response: serde_json::Value = serde_json::from_str(
            cached
                .response_json
                .as_deref()
                .expect("positive label row should retain provider JSON"),
        )
        .expect("positive label row JSON should parse");
        assert_eq!(response["label"], expected_label);
    }
}

#[tokio::test]
async fn backfill_albums_backfill_cache_persistence_surfaces_selective_failure_and_rescans_success()
{
    let db_conn = create_single_track_test_db("album-cache-accept", "/tmp/album-cache-accept.flac");
    insert_test_track(
        &db_conn,
        "album-cache-reject",
        "Album Cache Reject",
        "g1",
        "/tmp/album-cache-reject.flac",
    );
    db_conn
        .execute_batch(
            "UPDATE djmdArtist SET Name = 'Album Partial Artist' WHERE ID = 'a1';
             UPDATE djmdContent
             SET Title = 'Album Cache Accept', AlbumID = NULL
             WHERE ID = 'album-cache-accept';
             UPDATE djmdContent SET AlbumID = NULL WHERE ID = 'album-cache-reject';",
        )
        .expect("album partial fixture should need enrichment");
    let (server, _store_dir, _store_path) = create_enrich_cache_writer_test_server(db_conn);
    server
        .cache_store_conn()
        .expect("album partial store should open")
        .execute_batch(
            "CREATE TRIGGER fail_selected_album_cache
             BEFORE INSERT ON enrichment_cache
             WHEN NEW.provider = 'bandcamp' AND NEW.query_title = 'album cache reject'
             BEGIN
                 SELECT RAISE(FAIL, 'selected album cache failure');
             END;",
        )
        .expect("album partial failure trigger should install");
    for (title, album) in [
        ("Album Cache Accept", "Persisted Album"),
        ("Album Cache Reject", "Rejected Album"),
    ] {
        set_test_bandcamp_lookup_override(
            "Album Partial Artist",
            title,
            Ok(Some(crate::adapters::providers::bandcamp::BandcampResult {
                track_title: title.into(),
                artist_name: "Album Partial Artist".into(),
                release_date: Some("2024-01-01".into()),
                label: Some("Album Partial Label".into()),
                tags: vec![],
                album: Some(album.into()),
                cover_image: None,
                bandcamp_url: "https://example.bandcamp.com/track/partial".into(),
                score: 100,
            })),
        );
    }

    let result = tokio::time::timeout(
        Duration::from_secs(5),
        server.backfill_albums(Parameters(BackfillAlbumsParams {
            dry_run: Some(false),
            auto_enrich: Some(true),
        })),
    )
    .await
    .expect("album partial handler should finish within five seconds")
    .expect("album partial handler should return structured partial success");
    let payload = extract_json(&result);
    assert_eq!(payload["auto_enriched"], 2);
    assert_eq!(payload["auto_enrichment"]["requested"], 2);
    assert_eq!(payload["auto_enrichment"]["matched"], 2);
    assert_eq!(payload["auto_enrichment"]["cache_writes_succeeded"], 1);
    assert_eq!(payload["auto_enrichment"]["cache_writes_failed"], 1);
    assert_eq!(payload["auto_enrichment"]["writer_failed"], 0);
    assert_eq!(payload["auto_enrichment"]["operation_failed"], true);
    let failure = &payload["auto_enrichment"]["failures"][0];
    assert_eq!(failure["provider"], "bandcamp");
    assert_eq!(failure["normalized_title"], "album cache reject");
    assert_eq!(failure["kind"], "cache_write_failed");
    assert_eq!(payload["staged"], 1);
    assert_eq!(
        server
            .context
            .mutation
            .changes
            .get("album-cache-accept")
            .expect("persisted album should be re-scanned")
            .album
            .as_deref(),
        Some("Persisted Album")
    );
    assert!(
        server
            .context
            .mutation
            .changes
            .get("album-cache-reject")
            .is_none()
    );
}

#[tokio::test]
async fn backfill_years_backfill_cache_persistence_rejects_every_key_when_writer_open_fails() {
    let db_conn = create_single_track_test_db(
        "year-writer-open-failure",
        "/tmp/year-writer-open-failure.flac",
    );
    db_conn
        .execute_batch(
            "UPDATE djmdArtist SET Name = 'Year Open Failure Artist' WHERE ID = 'a1';
             UPDATE djmdContent
             SET Title = 'Year Open Failure Title', ReleaseYear = 0
             WHERE ID = 'year-writer-open-failure';",
        )
        .expect("year writer-open fixture should need enrichment");
    let store_dir = tempfile::tempdir().expect("year writer-open store directory should create");
    let usable_path = store_dir.path().join("usable.sqlite3");
    let usable_path_string = usable_path.to_string_lossy().to_string();
    let store_conn = store::open(&usable_path_string).expect("usable year store should open");
    let server = create_server_with_store_path(
        db_conn,
        store_conn,
        default_http_client_for_tests(),
        Some(store_dir.path().to_string_lossy().to_string()),
    );
    set_test_bandcamp_lookup_override(
        "Year Open Failure Artist",
        "Year Open Failure Title",
        Ok(None),
    );
    set_test_musicbrainz_lookup_override(
        "Year Open Failure Artist",
        "Year Open Failure Title",
        Ok(None),
    );

    let result = tokio::time::timeout(
        Duration::from_secs(5),
        server.backfill_years(Parameters(BackfillYearsParams {
            dry_run: Some(true),
            auto_enrich: Some(true),
        })),
    )
    .await
    .expect("year writer-open handler should finish within five seconds")
    .expect("year writer-open handler should return structured partial success");
    let payload = extract_json(&result);
    assert_eq!(payload["auto_enriched"], 0);
    assert_eq!(payload["auto_enrichment"]["requested"], 2);
    assert_eq!(payload["auto_enrichment"]["no_match"], 2);
    assert_eq!(payload["auto_enrichment"]["cache_writes_succeeded"], 0);
    assert_eq!(payload["auto_enrichment"]["cache_writes_failed"], 2);
    assert_eq!(payload["auto_enrichment"]["writer_failed"], 1);
    assert_eq!(payload["auto_enrichment"]["operation_failed"], true);
    let failures = payload["auto_enrichment"]["failures"]
        .as_array()
        .expect("year writer-open failures should be an array");
    assert_eq!(failures.len(), 2);
    assert!(failures.iter().all(|failure| {
        failure["kind"] == "writer_open_failed"
            && failure["normalized_artist"] == "year open failure artist"
            && failure["normalized_title"] == "year open failure title"
    }));
    let connection = store::open(&usable_path_string).expect("usable year store should reopen");
    for provider in ["bandcamp", "musicbrainz"] {
        assert!(
            store::get_enrichment(
                &connection,
                provider,
                "year open failure artist",
                "year open failure title",
                None,
                false,
            )
            .expect("usable year cache should read")
            .is_none()
        );
    }
}

#[tokio::test]
async fn backfill_labels_backfill_cache_persistence_surfaces_selective_provider_failure() {
    let db_conn =
        create_single_track_test_db("label-cache-selective", "/tmp/label-cache-selective.flac");
    db_conn
        .execute_batch(
            "UPDATE djmdArtist SET Name = 'Label Selective Artist' WHERE ID = 'a1';
             UPDATE djmdContent
             SET Title = 'Label Selective Title', LabelID = NULL
             WHERE ID = 'label-cache-selective';",
        )
        .expect("label selective fixture should need enrichment");
    let (server, _store_dir, _store_path) = create_enrich_cache_writer_test_server(db_conn);
    server
        .cache_store_conn()
        .expect("label selective store should open")
        .execute_batch(
            "CREATE TRIGGER fail_selected_label_cache
             BEFORE INSERT ON enrichment_cache
             WHEN NEW.provider = 'bandcamp' AND NEW.query_title = 'label selective title'
             BEGIN
                 SELECT RAISE(FAIL, 'selected label cache failure');
             END;",
        )
        .expect("label selective failure trigger should install");
    set_test_musicbrainz_lookup_override(
        "Label Selective Artist",
        "Label Selective Title",
        Ok(Some(
            crate::adapters::providers::musicbrainz::MusicBrainzResult {
                recording_title: "Label Selective Title".into(),
                artist: "Label Selective Artist".into(),
                first_release_date: Some("2024-01-01".into()),
                label: Some("Persisted Label".into()),
                score: 100,
            },
        )),
    );
    set_test_bandcamp_lookup_override(
        "Label Selective Artist",
        "Label Selective Title",
        Ok(Some(crate::adapters::providers::bandcamp::BandcampResult {
            track_title: "Label Selective Title".into(),
            artist_name: "Label Selective Artist".into(),
            release_date: Some("2024-01-01".into()),
            label: Some("Rejected Label".into()),
            tags: vec![],
            album: Some("Encoded Paths".into()),
            cover_image: None,
            bandcamp_url: "https://example.bandcamp.com/track/label-selective".into(),
            score: 100,
        })),
    );

    let result = tokio::time::timeout(
        Duration::from_secs(5),
        server.backfill_labels(Parameters(BackfillLabelsParams {
            dry_run: Some(false),
            auto_enrich: Some(true),
            max_conflicts: None,
            conflict_offset: None,
        })),
    )
    .await
    .expect("label selective handler should finish within five seconds")
    .expect("label selective handler should return structured partial success");
    let payload = extract_json(&result);
    assert_eq!(payload["auto_enriched"], 2);
    assert_eq!(payload["auto_enriched_by_provider"]["musicbrainz"], 1);
    assert_eq!(payload["auto_enriched_by_provider"]["bandcamp"], 1);
    assert_eq!(payload["auto_enrichment"]["cache_writes_succeeded"], 1);
    assert_eq!(payload["auto_enrichment"]["cache_writes_failed"], 1);
    assert_eq!(payload["auto_enrichment"]["operation_failed"], true);
    assert_eq!(
        payload["auto_enrichment"]["failures"][0]["provider"],
        "bandcamp"
    );
    assert_eq!(
        payload["auto_enrichment"]["failures"][0]["kind"],
        "cache_write_failed"
    );
    assert_eq!(payload["staged"], 1);
    assert_eq!(
        server
            .context
            .mutation
            .changes
            .get("label-cache-selective")
            .expect("persisted label should be re-scanned")
            .label
            .as_deref(),
        Some("Persisted Label")
    );
}

#[tokio::test]
async fn backfill_labels_backfill_cache_persistence_keeps_provider_errors_retryable_and_caches_no_match()
 {
    let db_conn = create_single_track_test_db(
        "labels-auto-enrich-error",
        "/music/labels-auto-enrich-error.flac",
    );
    db_conn
        .execute_batch(
            "UPDATE djmdArtist SET Name = 'Auto Error Artist' WHERE ID = 'a1';
             UPDATE djmdContent
             SET Title = 'Auto Error Title', LabelID = NULL
             WHERE ID = 'labels-auto-enrich-error';",
        )
        .expect("label fixture should become unlabeled");
    let (server, _store_dir, _store_path) = create_enrich_cache_writer_test_server(db_conn);

    set_test_musicbrainz_lookup_override(
        "Auto Error Artist",
        "Auto Error Title",
        Err("synthetic MusicBrainz failure".into()),
    );
    set_test_bandcamp_lookup_override("Auto Error Artist", "Auto Error Title", Ok(None));

    let result = tokio::time::timeout(
        Duration::from_secs(5),
        server.backfill_labels(Parameters(BackfillLabelsParams {
            dry_run: Some(true),
            auto_enrich: Some(true),
            max_conflicts: None,
            conflict_offset: None,
        })),
    )
    .await
    .expect("label failure handler should finish within five seconds")
    .expect("provider failures should not fail the whole label pass");
    let payload = extract_json(&result);

    assert_eq!(payload["auto_enriched"], 0);
    assert_eq!(payload["auto_enriched_by_provider"]["musicbrainz"], 0);
    assert_eq!(payload["auto_enriched_by_provider"]["bandcamp"], 0);
    assert_eq!(payload["auto_enrichment"]["requested"], 2);
    assert_eq!(payload["auto_enrichment"]["no_match"], 1);
    assert_eq!(payload["auto_enrichment"]["lookup_failed"], 1);
    assert_eq!(payload["auto_enrichment"]["cache_writes_succeeded"], 1);
    assert_eq!(payload["auto_enrichment"]["cache_writes_failed"], 0);
    assert_eq!(payload["auto_enrichment"]["operation_failed"], true);
    assert_eq!(
        payload["auto_enrichment"]["by_provider"]["musicbrainz"]["requested"],
        1
    );
    assert_eq!(
        payload["auto_enrichment"]["by_provider"]["musicbrainz"]["lookup_failed"],
        1
    );
    assert_eq!(
        payload["auto_enrichment"]["by_provider"]["bandcamp"]["requested"],
        1
    );
    assert_eq!(
        payload["auto_enrichment"]["by_provider"]["bandcamp"]["no_match"],
        1
    );
    assert_eq!(
        payload["auto_enrichment"]["failures"][0]["provider"],
        "musicbrainz"
    );
    assert_eq!(
        payload["auto_enrichment"]["failures"][0]["normalized_title"],
        "auto error title"
    );
    assert_eq!(
        payload["auto_enrichment"]["failures"][0]["kind"],
        "lookup_failed"
    );
    assert_eq!(payload["staged"], 0);

    let store_conn = server.cache_store_conn().expect("test store should open");
    let norm_artist = crate::domain::metadata::normalize_for_matching("Auto Error Artist");
    let norm_title = crate::domain::metadata::normalize_for_matching("Auto Error Title");
    assert!(
        store::get_enrichment(
            &store_conn,
            "musicbrainz",
            &norm_artist,
            &norm_title,
            None,
            false,
        )
        .expect("MusicBrainz cache should be readable")
        .is_none(),
        "provider errors must remain retryable"
    );
    let bandcamp = store::get_enrichment(
        &store_conn,
        "bandcamp",
        &norm_artist,
        &norm_title,
        None,
        false,
    )
    .expect("Bandcamp cache should be readable")
    .expect("completed no-match should be durable");
    assert_eq!(bandcamp.match_quality.as_deref(), Some("none"));
    assert!(bandcamp.response_json.is_none());
}

#[tokio::test]
async fn metadata_backfill_cancellation_aborts_provider_work_and_quiesces_writer() {
    use crate::application::metadata::enrichment::{
        MetadataEnrichmentProvider, install_test_lookup_pause, metadata_writer_active_for_test,
    };

    let db_conn = create_single_track_test_db(
        "metadata-cancellation-track",
        "/tmp/metadata-cancellation-track.flac",
    );
    db_conn
        .execute_batch(
            "UPDATE djmdArtist SET Name = 'Cancellation Artist' WHERE ID = 'a1';
             UPDATE djmdContent
             SET Title = 'Cancellation Title', AlbumID = NULL
             WHERE ID = 'metadata-cancellation-track';",
        )
        .expect("metadata cancellation fixture should need enrichment");
    let (server, _store_dir, store_path) = create_enrich_cache_writer_test_server(db_conn);
    set_test_bandcamp_lookup_override(
        "Cancellation Artist",
        "Cancellation Title",
        Ok(Some(crate::adapters::providers::bandcamp::BandcampResult {
            track_title: "Cancellation Title".into(),
            artist_name: "Cancellation Artist".into(),
            release_date: Some("2024-01-01".into()),
            label: Some("Cancellation Label".into()),
            tags: vec![],
            album: Some("Must Not Persist".into()),
            cover_image: None,
            bandcamp_url: "https://example.bandcamp.com/track/cancel".into(),
            score: 100,
        })),
    );
    let norm_artist = crate::domain::metadata::normalize_for_matching("Cancellation Artist");
    let norm_title = crate::domain::metadata::normalize_for_matching("Cancellation Title");
    let (pause_guard, reached, _release) = install_test_lookup_pause(
        MetadataEnrichmentProvider::Bandcamp,
        norm_artist.clone(),
        norm_title.clone(),
    );

    let task_server = server.clone();
    let mut handler = tokio::spawn(async move {
        task_server
            .backfill_albums(Parameters(BackfillAlbumsParams {
                dry_run: Some(false),
                auto_enrich: Some(true),
            }))
            .await
    });
    if tokio::time::timeout(Duration::from_secs(5), reached.notified())
        .await
        .is_err()
    {
        handler.abort();
        drop(pause_guard);
        assert!(
            tokio::time::timeout(Duration::from_secs(5), &mut handler)
                .await
                .is_ok(),
            "timed-out metadata handler cleanup should join within five seconds"
        );
        panic!("metadata provider did not reach the cancellation barrier");
    }
    if tokio::time::timeout(Duration::from_secs(5), async {
        while !metadata_writer_active_for_test(&store_path) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .is_err()
    {
        handler.abort();
        drop(pause_guard);
        assert!(
            tokio::time::timeout(Duration::from_secs(5), &mut handler)
                .await
                .is_ok(),
            "inactive metadata writer cleanup should join within five seconds"
        );
        panic!("metadata writer did not become active before cancellation");
    }

    handler.abort();
    drop(pause_guard);
    let cancelled = tokio::time::timeout(Duration::from_secs(5), &mut handler)
        .await
        .expect("metadata handler cancellation should join within five seconds")
        .expect_err("metadata handler should be cancelled");
    assert!(cancelled.is_cancelled());

    tokio::time::timeout(Duration::from_secs(5), async {
        while metadata_writer_active_for_test(&store_path) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("metadata writer should quiesce within five seconds");

    let connection = server
        .cache_store_conn()
        .expect("metadata cancellation store should remain usable");
    assert!(
        store::get_enrichment(
            &connection,
            "bandcamp",
            &norm_artist,
            &norm_title,
            None,
            false,
        )
        .expect("metadata cancellation cache should read")
        .is_none(),
        "cancelled provider must not persist a cache row after writer quiescence"
    );
}

#[tokio::test]
async fn metadata_auto_enrichment_output_is_conditional_and_reports_zero_work() {
    let db_conn = create_single_track_test_db(
        "metadata-output-zero-work",
        "/tmp/metadata-output-zero-work.flac",
    );
    let (server, _store_dir, _store_path) = create_enrich_cache_writer_test_server(db_conn);

    let without_auto_enrich = tokio::time::timeout(
        Duration::from_secs(5),
        server.backfill_labels(Parameters(BackfillLabelsParams {
            dry_run: Some(true),
            auto_enrich: Some(false),
            max_conflicts: None,
            conflict_offset: None,
        })),
    )
    .await
    .expect("non-auto label handler should finish within five seconds")
    .expect("label output without auto enrichment should succeed");
    let without_auto_enrich = extract_json(&without_auto_enrich);
    assert!(without_auto_enrich.get("auto_enrichment").is_none());
    assert!(without_auto_enrich.get("auto_enriched").is_none());

    let labels = tokio::time::timeout(
        Duration::from_secs(5),
        server.backfill_labels(Parameters(BackfillLabelsParams {
            dry_run: Some(true),
            auto_enrich: Some(true),
            max_conflicts: None,
            conflict_offset: None,
        })),
    )
    .await
    .expect("zero-work label handler should finish within five seconds")
    .expect("zero-work label auto enrichment should succeed");
    let years = tokio::time::timeout(
        Duration::from_secs(5),
        server.backfill_years(Parameters(BackfillYearsParams {
            dry_run: Some(true),
            auto_enrich: Some(true),
        })),
    )
    .await
    .expect("zero-work year handler should finish within five seconds")
    .expect("zero-work year auto enrichment should succeed");
    let albums = tokio::time::timeout(
        Duration::from_secs(5),
        server.backfill_albums(Parameters(BackfillAlbumsParams {
            dry_run: Some(true),
            auto_enrich: Some(true),
        })),
    )
    .await
    .expect("zero-work album handler should finish within five seconds")
    .expect("zero-work album auto enrichment should succeed");

    for (name, result) in [("labels", labels), ("years", years), ("albums", albums)] {
        let payload = extract_json(&result);
        let report = &payload["auto_enrichment"];
        assert_eq!(report["requested"], 0, "{name} requested count");
        assert_eq!(report["matched"], 0, "{name} matched count");
        assert_eq!(report["no_match"], 0, "{name} no-match count");
        assert_eq!(report["cache_writes_succeeded"], 0, "{name} write count");
        assert_eq!(report["operation_failed"], false, "{name} failure flag");
        assert_eq!(report["failures"], serde_json::json!([]));
        assert_eq!(report["failures_truncated"], false);
        assert!(report["by_provider"].get("bandcamp").is_some());
        assert!(report["by_provider"].get("musicbrainz").is_some());
        assert_eq!(payload["auto_enriched"], 0);
    }
}

#[test]
fn metadata_auto_enrichment_output_schema_exposes_typed_label_report() {
    fn contains_fields(
        root: &serde_json::Value,
        value: &serde_json::Value,
        fields: &[&str],
    ) -> bool {
        if let Some(reference) = value.get("$ref").and_then(serde_json::Value::as_str)
            && let Some(target) = root.pointer(reference.trim_start_matches('#'))
        {
            return contains_fields(root, target, fields);
        }
        if let Some(properties) = value
            .get("properties")
            .and_then(serde_json::Value::as_object)
            && fields.iter().all(|field| properties.contains_key(*field))
        {
            return true;
        }
        match value {
            serde_json::Value::Array(values) => values
                .iter()
                .any(|value| contains_fields(root, value, fields)),
            serde_json::Value::Object(values) => values
                .values()
                .any(|value| contains_fields(root, value, fields)),
            _ => false,
        }
    }

    let tool = ReklawdboxServer::build_tool_router()
        .list_all()
        .into_iter()
        .find(|tool| tool.name.as_ref() == "backfill_labels")
        .expect("backfill_labels should exist in the live router");
    let schema = serde_json::to_value(
        tool.output_schema
            .as_ref()
            .expect("backfill_labels should advertise outputSchema"),
    )
    .expect("backfill_labels output schema should serialize");
    let root_properties = schema["properties"]
        .as_object()
        .expect("backfill_labels output schema should expose root properties");
    let auto_enrichment = root_properties
        .get("auto_enrichment")
        .expect("backfill_labels output schema should expose auto_enrichment");
    assert!(
        !schema["required"]
            .as_array()
            .expect("backfill_labels required fields should be an array")
            .iter()
            .any(|field| field == "auto_enrichment"),
        "auto_enrichment is conditional on auto_enrich=true"
    );
    assert!(
        contains_fields(
            &schema,
            auto_enrichment,
            &[
                "operation_failed",
                "requested",
                "matched",
                "no_match",
                "lookup_failed",
                "cache_writes_succeeded",
                "cache_writes_failed",
                "serialization_failed",
                "worker_failed",
                "writer_failed",
                "by_provider",
                "failures",
                "failures_truncated",
            ],
        ),
        "unexpected auto_enrichment schema: {auto_enrichment:#}"
    );
}

#[test]
fn color_input_public_contract() {
    let changes = ChangeManager::new();
    handle_update_tracks(
        &changes,
        UpdateTracksParams {
            changes: vec![TrackChangeInput {
                track_id: "color-name".to_owned(),
                genre: None,
                comments: None,
                rating: None,
                color: Some("turquoise".to_owned()),
                label: None,
                year: None,
                album: None,
            }],
        },
    )
    .expect("a canonical color name should validate without a database");
    assert_eq!(
        changes
            .get("color-name")
            .expect("accepted color should be staged")
            .color
            .as_deref(),
        Some("Turquoise"),
        "accepted color names should be canonicalized"
    );

    let error = handle_update_tracks(
        &ChangeManager::new(),
        UpdateTracksParams {
            changes: vec![TrackChangeInput {
                track_id: "color-hex".to_owned(),
                genre: None,
                comments: None,
                rating: None,
                color: Some("0x25FDE9".to_owned()),
                label: None,
                year: None,
                album: None,
            }],
        },
    )
    .expect_err("serialized XML hex must not be accepted as a tool input");
    let message = format!("{error:?}");
    for name in [
        "Blue",
        "Green",
        "Lemon",
        "Orange",
        "Red",
        "Rose",
        "Turquoise",
        "Violet",
    ] {
        assert!(
            message.contains(name),
            "invalid color guidance should include {name}: {message}"
        );
    }

    let mut track = make_test_track("color-xml", "House", 124.0, "8A");
    track.color_code = crate::domain::metadata::color_name_to_code("Turquoise")
        .expect("canonical color should have an XML code");
    let xml = crate::adapters::rekordbox::xml::generate_xml(&[track]);
    assert!(
        xml.contains("Colour=\"0x25FDE9\""),
        "canonical color integers should serialize as uppercase 0xRRGGBB"
    );
}

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

#[tokio::test]
#[cfg(unix)]
#[allow(clippy::await_holding_lock)]
async fn write_xml_backup_timeout_restores_state_and_releases_lock() {
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
    let fixture = write_hanging_backup_fixture(backup_dir.path(), true);
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
    assert!(
        error
            .message
            .contains("pre-op pre-operation backup timed out after 250ms"),
        "unexpected timeout error: {}",
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

#[tokio::test]
async fn update_tracks_stages_changes() {
    let server = ReklawdboxServer::new(None);
    let known_genre = genre::GENRES
        .first()
        .copied()
        .unwrap_or("House")
        .to_string();

    let result = server
        .update_tracks(Parameters(UpdateTracksParams {
            changes: vec![TrackChangeInput {
                track_id: "test-track-1".to_string(),
                genre: Some(known_genre),
                comments: Some("staged by test".to_string()),
                rating: Some(4),
                color: None,
                label: None,
                year: None,
                album: None,
            }],
        }))
        .await
        .expect("update_tracks should succeed");

    let payload = extract_json(&result);
    assert_eq!(
        payload
            .get("staged")
            .and_then(serde_json::Value::as_u64)
            .expect("staged should be present"),
        1
    );
    assert_eq!(
        payload
            .get("total_pending")
            .and_then(serde_json::Value::as_u64)
            .expect("total_pending should be present"),
        1
    );
    assert!(
        payload.get("changes").is_none(),
        "update_tracks should not echo changes back"
    );
}

#[tokio::test]
async fn update_tracks_via_router_warns_non_taxonomy_genre() {
    let result = call_tool_via_router(
        "update_tracks",
        serde_json::json!({
            "changes": [{
                "track_id": "router-test-track-1",
                "genre": "NotInTaxonomy"
            }]
        })
        .as_object()
        .cloned(),
    )
    .await;

    let payload = extract_json(&result);
    assert_eq!(
        payload
            .get("staged")
            .and_then(serde_json::Value::as_u64)
            .expect("staged should be present"),
        1
    );
    let warnings = payload
        .get("warnings")
        .and_then(serde_json::Value::as_array)
        .expect("warnings should be present for non-taxonomy genre");
    assert!(
        !warnings.is_empty(),
        "warnings should include at least one non-taxonomy genre warning"
    );
}
