use super::support::{EnvVarGuard, backup_script_env_lock, write_executable_script};
use crate::domain::metadata::TrackChange;
use crate::mcp::metadata::WriteXmlParams;
use crate::mcp::server::ReklawdboxServer;
use std::time::Duration;

use rmcp::handler::server::wrapper::Parameters;

#[cfg(unix)]
pub(super) fn run_embedded_backup_script(
    args: &[&str],
    home: &std::path::Path,
    db_path: Option<&std::path::Path>,
    stdin: Option<&str>,
) -> std::process::Output {
    run_embedded_backup_script_with_temp_dir(args, home, db_path, stdin, &home.join("tmp"))
}

#[cfg(unix)]
pub(super) fn run_embedded_backup_script_with_temp_dir(
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
pub(super) fn child_output_text(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[cfg(unix)]
pub(super) fn backup_archives(home: &std::path::Path, prefix: &str) -> Vec<std::path::PathBuf> {
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
pub(super) fn tar_members(archive: &std::path::Path) -> Vec<String> {
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
pub(super) fn create_backup_archive_fixture(
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
