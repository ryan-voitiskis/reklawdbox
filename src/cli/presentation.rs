//! Human-facing CLI filesystem expansion and formatting helpers.

use std::path::{Path, PathBuf};

fn is_audio_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| {
            crate::audio::AUDIO_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str())
        })
}

pub(crate) fn expand_paths(paths: &[String], recursive: bool) -> Vec<PathBuf> {
    let mut result = Vec::new();
    for p in paths {
        let path = PathBuf::from(p);
        if path.is_dir() {
            collect_audio_files(&path, recursive, &mut result);
        } else {
            result.push(path);
        }
    }
    result
}

fn collect_audio_files(dir: &Path, recursive: bool, result: &mut Vec<PathBuf>) {
    match std::fs::read_dir(dir) {
        Ok(entries) => {
            let mut files = Vec::new();
            let mut subdirs = Vec::new();
            for entry in entries.filter_map(std::result::Result::ok) {
                let is_symlink = entry.file_type().is_ok_and(|ft| ft.is_symlink());
                let path = entry.path();
                if path.is_file() && is_audio_file(&path) {
                    files.push(path);
                } else if recursive && path.is_dir() && !is_symlink {
                    subdirs.push(path);
                }
            }
            files.sort();
            result.extend(files);
            subdirs.sort();
            for subdir in subdirs {
                collect_audio_files(&subdir, true, result);
            }
        }
        Err(e) => {
            eprintln!(
                "Warning: skipping unreadable directory {}: {e}",
                dir.display()
            );
        }
    }
}

/// "album_artist" -> "Album Artist", "bpm" -> "BPM", etc.
pub(crate) fn display_field_name(field: &str) -> String {
    field
        .split('_')
        .map(|word| match word {
            "bpm" => "BPM".to_string(),
            _ => {
                let mut chars = word.chars();
                match chars.next() {
                    Some(c) => {
                        let upper: String = c.to_uppercase().collect();
                        format!("{upper}{}", chars.as_str())
                    }
                    None => String::new(),
                }
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
