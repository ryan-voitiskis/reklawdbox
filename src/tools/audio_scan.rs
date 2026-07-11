use rmcp::ErrorData as McpError;

use crate::audio;
use crate::audio::AUDIO_EXTENSIONS;

pub(super) fn scan_audio_directory(
    dir: &str,
    recursive: bool,
    glob_pattern: Option<&str>,
) -> Result<Vec<String>, String> {
    let dir_path = std::path::Path::new(dir);
    if !dir_path.is_dir() {
        return Err(format!("Not a directory: {dir}"));
    }

    let glob_matcher = match glob_pattern {
        Some(pattern) => {
            let glob = globset::GlobBuilder::new(pattern)
                .literal_separator(true)
                .case_insensitive(true)
                .build()
                .map_err(|e| format!("Invalid glob pattern \"{pattern}\": {e}"))?;
            Some(glob.compile_matcher())
        }
        None => None,
    };

    let canonical_root = std::fs::canonicalize(dir_path).map_err(|e| {
        format!(
            "Failed to canonicalize directory {}: {e}",
            dir_path.display()
        )
    })?;
    let mut visited_directories = std::collections::HashSet::from([canonical_root]);
    let mut files = Vec::new();
    let mut dirs_to_scan = vec![dir_path.to_path_buf()];

    while let Some(current_dir) = dirs_to_scan.pop() {
        let entries = std::fs::read_dir(&current_dir)
            .map_err(|e| format!("Failed to read directory {}: {e}", current_dir.display()))?;

        for entry in entries {
            let entry = entry.map_err(|e| format!("Directory entry error: {e}"))?;
            let path = entry.path();
            let file_type = entry
                .file_type()
                .map_err(|e| format!("Directory entry error: {e}"))?;

            if file_type.is_dir() {
                if recursive {
                    let canonical_path = std::fs::canonicalize(&path).map_err(|e| {
                        format!("Failed to canonicalize directory {}: {e}", path.display())
                    })?;
                    if visited_directories.insert(canonical_path) {
                        dirs_to_scan.push(path);
                    }
                }
                continue;
            }

            let is_file = file_type.is_file()
                || (file_type.is_symlink()
                    && std::fs::metadata(&path).is_ok_and(|metadata| metadata.is_file()));
            if !is_file {
                continue;
            }

            // Audio extension check applies even when glob is set
            let is_audio = path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| AUDIO_EXTENSIONS.contains(&e.to_lowercase().as_str()));
            if !is_audio {
                continue;
            }

            if let Some(ref matcher) = glob_matcher {
                let file_name = match path.file_name().and_then(|n| n.to_str()) {
                    Some(n) => n,
                    None => continue,
                };
                if !matcher.is_match(file_name) {
                    continue;
                }
            }

            files.push(path.display().to_string());
        }
    }

    files.sort();
    Ok(files)
}

pub(super) fn resolve_file_path(raw_path: &str) -> Result<String, McpError> {
    audio::resolve_audio_path(raw_path).map_err(|e| McpError::internal_error(e.to_string(), None))
}

#[cfg(test)]
mod tests {
    use super::scan_audio_directory;

    fn create_file(path: &std::path::Path) {
        std::fs::write(path, b"").expect("test audio file should be created");
    }

    fn path_string(path: &std::path::Path) -> String {
        path.display().to_string()
    }

    #[test]
    fn audio_scan_non_recursive_excludes_real_child_directory() {
        let root = tempfile::tempdir().expect("temporary scan root should be created");
        let nested = root.path().join("nested");
        std::fs::create_dir(&nested).expect("nested directory should be created");
        let root_audio = root.path().join("root.mp3");
        create_file(&root_audio);
        create_file(&nested.join("nested.wav"));

        let files = scan_audio_directory(
            root.path()
                .to_str()
                .expect("temporary path should be UTF-8"),
            false,
            None,
        )
        .expect("non-recursive scan should succeed");

        assert_eq!(files, vec![path_string(&root_audio)]);
    }

    #[test]
    fn audio_scan_recursive_returns_nested_files_once_and_sorted() {
        let root = tempfile::tempdir().expect("temporary scan root should be created");
        let nested = root.path().join("nested");
        let deeper = nested.join("deeper");
        std::fs::create_dir_all(&deeper).expect("nested directories should be created");
        let root_audio = root.path().join("z.mp3");
        let nested_audio = nested.join("a.flac");
        let deeper_audio = deeper.join("m.wav");
        create_file(&root_audio);
        create_file(&nested_audio);
        create_file(&deeper_audio);

        let files = scan_audio_directory(
            root.path()
                .to_str()
                .expect("temporary path should be UTF-8"),
            true,
            None,
        )
        .expect("recursive scan should succeed");
        let mut expected = vec![
            path_string(&root_audio),
            path_string(&nested_audio),
            path_string(&deeper_audio),
        ];
        expected.sort();

        assert_eq!(files, expected);
    }

    #[test]
    fn audio_scan_preserves_extension_and_glob_filtering() {
        let root = tempfile::tempdir().expect("temporary scan root should be created");
        let matching = root.path().join("MATCH.MP3");
        create_file(&matching);
        create_file(&root.path().join("other.wav"));
        create_file(&root.path().join("not-audio.mp3.txt"));

        let files = scan_audio_directory(
            root.path()
                .to_str()
                .expect("temporary path should be UTF-8"),
            false,
            Some("match.mp3"),
        )
        .expect("filtered scan should succeed");

        assert_eq!(files, vec![path_string(&matching)]);
    }

    #[cfg(unix)]
    const AUDIO_SCAN_CHILD_CASE_ENV: &str = "REKLAWDBOX_AUDIO_SCAN_CHILD_CASE";

    #[cfg(unix)]
    fn run_audio_scan_cycle_child(case: &str) {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().expect("temporary cycle root should be created");
        let root_audio = root.path().join("root.mp3");
        create_file(&root_audio);

        let expected = match case {
            "self" => {
                symlink(root.path(), root.path().join("self"))
                    .expect("self directory symlink should be created");
                vec![path_string(&root_audio)]
            }
            "parent" => {
                let nested = root.path().join("nested");
                std::fs::create_dir(&nested).expect("nested directory should be created");
                let nested_audio = nested.join("nested.wav");
                create_file(&nested_audio);
                symlink(root.path(), nested.join("parent"))
                    .expect("parent directory symlink should be created");
                let mut expected = vec![path_string(&root_audio), path_string(&nested_audio)];
                expected.sort();
                expected
            }
            other => panic!("unknown audio scan child case: {other}"),
        };

        let files = scan_audio_directory(
            root.path()
                .to_str()
                .expect("temporary path should be UTF-8"),
            true,
            None,
        )
        .expect("cycle scan should succeed");
        assert_eq!(files, expected);
    }

    #[cfg(unix)]
    #[test]
    fn audio_scan_symlink_cycle_child() {
        let Ok(case) = std::env::var(AUDIO_SCAN_CHILD_CASE_ENV) else {
            return;
        };
        run_audio_scan_cycle_child(&case);
    }

    #[cfg(unix)]
    fn assert_audio_scan_cycle_case_is_bounded(case: &str) {
        use std::process::Command;
        use std::time::{Duration, Instant};

        let executable = std::env::current_exe().expect("current test executable should resolve");
        let mut child = Command::new(executable)
            .arg("tools::audio_scan::tests::audio_scan_symlink_cycle_child")
            .arg("--exact")
            .arg("--nocapture")
            .env(AUDIO_SCAN_CHILD_CASE_ENV, case)
            .spawn()
            .expect("cycle child process should start");
        let deadline = Instant::now() + Duration::from_secs(5);

        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    assert!(
                        status.success(),
                        "audio scan cycle child for {case:?} failed with {status}"
                    );
                    return;
                }
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Ok(None) => {
                    let kill_result = child.kill();
                    let reap_result = child.wait();
                    panic!(
                        "audio scan cycle child for {case:?} exceeded five seconds; \
                         kill={kill_result:?}, reap={reap_result:?}"
                    );
                }
                Err(error) => {
                    let kill_result = child.kill();
                    let reap_result = child.wait();
                    panic!(
                        "failed to inspect audio scan cycle child for {case:?}: {error}; \
                         kill={kill_result:?}, reap={reap_result:?}"
                    );
                }
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn audio_scan_self_directory_symlink_terminates_without_duplicates() {
        assert_audio_scan_cycle_case_is_bounded("self");
    }

    #[cfg(unix)]
    #[test]
    fn audio_scan_parent_directory_symlink_terminates_without_duplicates() {
        assert_audio_scan_cycle_case_is_bounded("parent");
    }

    #[cfg(unix)]
    #[test]
    fn audio_scan_child_directory_aliases_are_skipped() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().expect("temporary scan root should be created");
        let real = root.path().join("real");
        std::fs::create_dir(&real).expect("real directory should be created");
        let audio = real.join("track.mp3");
        create_file(&audio);
        symlink(&real, root.path().join("alias-one"))
            .expect("first directory alias should be created");
        symlink(&real, root.path().join("alias-two"))
            .expect("second directory alias should be created");

        let files = scan_audio_directory(
            root.path()
                .to_str()
                .expect("temporary path should be UTF-8"),
            true,
            None,
        )
        .expect("alias scan should succeed");

        assert_eq!(files, vec![path_string(&audio)]);
    }

    #[cfg(unix)]
    #[test]
    fn audio_scan_regular_audio_file_symlink_is_preserved() {
        use std::os::unix::fs::symlink;

        let container = tempfile::tempdir().expect("temporary container should be created");
        let root = container.path().join("scan");
        std::fs::create_dir(&root).expect("scan root should be created");
        let target = container.path().join("target.flac");
        create_file(&target);
        let link = root.join("linked.flac");
        symlink(&target, &link).expect("audio file symlink should be created");

        let files = scan_audio_directory(
            root.to_str().expect("temporary path should be UTF-8"),
            true,
            None,
        )
        .expect("file symlink scan should succeed");

        assert_eq!(files, vec![path_string(&link)]);
    }

    #[cfg(unix)]
    #[test]
    fn audio_scan_broken_symlink_is_ignored() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().expect("temporary scan root should be created");
        symlink(
            root.path().join("missing.mp3"),
            root.path().join("broken.mp3"),
        )
        .expect("broken symlink should be created");

        let files = scan_audio_directory(
            root.path()
                .to_str()
                .expect("temporary path should be UTF-8"),
            true,
            None,
        )
        .expect("broken symlink scan should succeed");

        assert!(files.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn audio_scan_explicit_root_symlink_is_traversed_but_child_links_are_skipped() {
        use std::os::unix::fs::symlink;

        let container = tempfile::tempdir().expect("temporary container should be created");
        let real_root = container.path().join("real-root");
        let external = container.path().join("external");
        std::fs::create_dir(&real_root).expect("real root should be created");
        std::fs::create_dir(&external).expect("external directory should be created");
        let root_audio = real_root.join("root.mp3");
        create_file(&root_audio);
        create_file(&external.join("external.wav"));
        symlink(&external, real_root.join("child-link"))
            .expect("child directory symlink should be created");
        let root_link = container.path().join("root-link");
        symlink(&real_root, &root_link).expect("root directory symlink should be created");

        let files = scan_audio_directory(
            root_link.to_str().expect("temporary path should be UTF-8"),
            true,
            None,
        )
        .expect("root symlink scan should succeed");

        assert_eq!(files, vec![path_string(&root_link.join("root.mp3"))]);
    }
}
