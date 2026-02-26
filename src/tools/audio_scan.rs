use rmcp::ErrorData as McpError;

use crate::audio;
use crate::audio::AUDIO_EXTENSIONS;

/// Scan a directory for audio files, optionally recursive, with optional glob filter.
pub(super) fn scan_audio_directory(
    dir: &str,
    recursive: bool,
    glob_pattern: Option<&str>,
) -> Result<Vec<String>, String> {
    let dir_path = std::path::Path::new(dir);
    if !dir_path.is_dir() {
        return Err(format!("Not a directory: {dir}"));
    }

    // Compile glob matcher if a pattern was provided
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

    let mut files = Vec::new();
    let mut dirs_to_scan = vec![dir_path.to_path_buf()];

    while let Some(current_dir) = dirs_to_scan.pop() {
        let entries = std::fs::read_dir(&current_dir)
            .map_err(|e| format!("Failed to read directory {}: {e}", current_dir.display()))?;

        for entry in entries {
            let entry = entry.map_err(|e| format!("Directory entry error: {e}"))?;
            let path = entry.path();

            if path.is_dir() && recursive {
                dirs_to_scan.push(path);
                continue;
            }

            if !path.is_file() {
                continue;
            }

            // Must be an audio file regardless of glob
            let is_audio = path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| AUDIO_EXTENSIONS.contains(&e.to_lowercase().as_str()));
            if !is_audio {
                continue;
            }

            // Apply glob filter against the filename
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
    audio::resolve_audio_path(raw_path).map_err(super::resolve::err)
}
