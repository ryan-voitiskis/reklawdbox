//! Filename and album-context audit policy.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;

use super::AuditContext;

/// Check if a directory name represents a disc subdirectory (CD1, Disc 1, etc.).
/// Rejects false positives like "Disco Dreams", "Discovery", "CD" bare.
pub(crate) static DISC_SUBDIR_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?:CD\s*\d+|Dis[ck]\s*\d+)$").expect("DISC_SUBDIR_RE must compile")
});

pub(crate) fn is_disc_subdir(name: &str) -> bool {
    DISC_SUBDIR_RE.is_match(name)
}

/// Compiled regex matching tech-spec fragments in directory names:
/// format names (bracketed or bare), bit-depth/sample-rate combos with
/// optional fractional kHz and units, and standalone bit-depth labels.
pub(crate) static TECH_SPEC_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(?i)",
        r"\[(?:flac|wav|mp3|aiff|aac|alac|hi-?res)\]", // [FLAC], [Hi-Res], …
        r"|\b(?:flac|wav|mp3|aiff|aac|alac)\b",        // bare FLAC, wav, …
        r"|\b(?:16|24|32)\s?-\s?\d{2,3}(?:\.\d+)?\s*(?:khz|hz)?", // 24-44.1kHz
        r"|\b(?:16|24|32)\s?bit",                      // 24bit, 16 bit
        r"|\d{2,3}(?:\.\d+)?\s*(?:khz|hz)",            // 44.1kHz, 96kHz
    ))
    .expect("TECH_SPEC_RE must compile")
});

/// Regex for orphaned delimiters left after tech-spec stripping.
/// Matches `()`, `[]`, and variants containing only whitespace, hyphens, or
/// dots — residue left when the contents were entirely tech-spec fragments.
pub(crate) static ORPHAN_DELIM_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\([\s\-.]*\)|\[[\s\-.]*\]").expect("ORPHAN_DELIM_RE must compile")
});

/// Strip tech-spec fragments from a directory name for pattern matching.
/// Matching is case-insensitive but non-pattern text preserves its original
/// casing.
pub(crate) fn normalize_dir_name(name: &str) -> String {
    let result = TECH_SPEC_RE.replace_all(name, "");
    let result = ORPHAN_DELIM_RE.replace_all(&result, "");
    let result = result.split_whitespace().collect::<Vec<_>>().join(" ");
    let result = result
        .trim()
        .trim_matches(|c: char| c == '-' || c == '.')
        .trim();
    result.to_string()
}

/// Check if a directory name has a year suffix like `(2024)`.
pub(crate) fn has_year_suffix(name: &str) -> bool {
    let trimmed = name.trim_end();
    if trimmed.len() < 6 {
        return false;
    }
    let bytes = trimmed.as_bytes();
    if bytes[bytes.len() - 1] != b')' {
        return false;
    }
    if let Some(open) = trimmed.rfind('(') {
        let inside = &trimmed[open + 1..trimmed.len() - 1];
        let Some(year_prefix) = inside.get(..4) else {
            return false;
        };
        if year_prefix.bytes().all(|byte| byte.is_ascii_digit())
            && let Ok(year) = year_prefix.parse::<u16>()
        {
            return (1900..=2099).contains(&year);
        }
        false
    } else {
        false
    }
}

/// Check if a directory name contains a year range like `1977-1992` or
/// `1992–2014` (with either ASCII hyphen or en-dash).
pub(crate) fn has_year_range(name: &str) -> bool {
    static YEAR_RANGE_RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"\b(\d{4})[-–](\d{4})\b").expect("YEAR_RANGE_RE must compile")
    });
    YEAR_RANGE_RE.captures(name).is_some_and(|caps| {
        let a = caps[1].parse::<u16>().unwrap_or(0);
        let b = caps[2].parse::<u16>().unwrap_or(0);
        (1900..=2099).contains(&a) && (1900..=2099).contains(&b)
    })
}

/// Check if a directory name contains a bare (non-parenthesized) 4-digit year
/// like `Live in Tokyo - 1st December 2013` or `FM Broadcast August 1996`.
pub(crate) fn has_bare_year(name: &str) -> bool {
    static BARE_YEAR_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\b(19|20)\d{2}\b").expect("BARE_YEAR_RE must compile"));
    BARE_YEAR_RE.is_match(name)
}

/// Pre-pass: detect directories that contain 2+ files with track-number
/// prefixes (e.g. `01 `, `02-`, `03.`). These are album directories even
/// without a year suffix.
pub(crate) fn detect_album_dirs(paths: &[std::path::PathBuf]) -> HashSet<std::path::PathBuf> {
    static TRACK_PREFIX: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^\d{2,3}[\s.\-]").expect("TRACK_PREFIX must compile"));

    let mut counts: HashMap<std::path::PathBuf, usize> = HashMap::new();
    for path in paths {
        let parent = match path.parent() {
            Some(p) => p,
            None => continue,
        };
        let stem = match path.file_stem().and_then(|s| s.to_str()) {
            Some(s) => s,
            None => continue,
        };
        if TRACK_PREFIX.is_match(stem) {
            *counts.entry(parent.to_path_buf()).or_default() += 1;
        }
    }
    counts.retain(|_, count| *count >= 2);

    // Leaf-dir filter: a dir is an album dir only if no other audio-containing
    // dir is its direct child. This excludes mixed dirs (e.g. `lossy/` with
    // loose tracks AND artist subdirs) from being classified as album dirs.
    let child_parents: HashSet<std::path::PathBuf> = counts
        .keys()
        .filter_map(|dir| dir.parent().map(std::path::Path::to_path_buf))
        .collect();
    counts.retain(|dir, _| !child_parents.contains(dir));

    counts.into_keys().collect()
}

pub fn classify_track_context(
    path: &Path,
    album_dirs: &HashSet<std::path::PathBuf>,
) -> AuditContext {
    let parent = match path.parent() {
        Some(p) => p,
        None => return AuditContext::LooseTrack,
    };

    let dir_name = match parent.file_name().and_then(|n| n.to_str()) {
        Some(n) => n,
        None => return AuditContext::LooseTrack,
    };

    let (effective_parent, effective_dir_name) = if is_disc_subdir(dir_name) {
        match parent.parent() {
            Some(album_dir) => match album_dir.file_name().and_then(|n| n.to_str()) {
                Some(n) => (album_dir, n),
                None => return AuditContext::LooseTrack,
            },
            None => return AuditContext::LooseTrack,
        }
    } else {
        (parent, dir_name)
    };

    let normalized = normalize_dir_name(effective_dir_name);
    if has_year_suffix(&normalized) {
        return AuditContext::AlbumTrack;
    }

    // Check both effective parent and direct parent (disc subdir case).
    if album_dirs.contains(effective_parent) || album_dirs.contains(parent) {
        return AuditContext::AlbumTrack;
    }

    AuditContext::LooseTrack
}

/// Get the effective album directory name, climbing past disc subdirectories.
pub(crate) fn effective_album_dir_name(path: &Path) -> Option<(&Path, &str)> {
    let parent = path.parent()?;
    let dir_name = parent.file_name().and_then(|n| n.to_str())?;

    if is_disc_subdir(dir_name) {
        let album_dir = parent.parent()?;
        let album_name = album_dir.file_name().and_then(|n| n.to_str())?;
        Some((album_dir, album_name))
    } else {
        Some((parent, dir_name))
    }
}

/// Check whether any ancestor directory (up to 2 levels above the parent)
/// already has a year suffix, so nested subdirs don't redundantly flag
/// MISSING_YEAR_IN_DIR.
pub(crate) fn ancestor_has_year(path: &Path) -> bool {
    let parent = match path.parent() {
        Some(p) => p,
        None => return false,
    };
    let mut current = parent.parent();
    for _ in 0..2 {
        match current {
            Some(dir) => {
                if let Some(name) = dir.file_name().and_then(|n| n.to_str())
                    && (has_year_suffix(name) || has_year_suffix(&normalize_dir_name(name)))
                {
                    return true;
                }
                current = dir.parent();
            }
            None => break,
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Filename parsing
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct ParsedFilename {
    pub track_num: Option<String>,
    pub artist: Option<String>,
    pub title: Option<String>,
}

pub fn parse_filename(path: &Path, context: &AuditContext) -> ParsedFilename {
    let stem = match path.file_stem().and_then(|s| s.to_str()) {
        Some(s) => s,
        None => return ParsedFilename::default(),
    };

    match context {
        AuditContext::AlbumTrack => parse_album_filename(stem),
        AuditContext::LooseTrack => parse_loose_filename(stem),
    }
}

pub(crate) fn parse_album_filename(stem: &str) -> ParsedFilename {
    let bytes = stem.as_bytes();
    if bytes.len() < 3 {
        return ParsedFilename::default();
    }

    // Track numbers and disc prefixes are always ASCII — bail early if first
    // chars are not ASCII (avoids panicking on multi-byte UTF-8).
    if !bytes[0].is_ascii() || !bytes[1].is_ascii() {
        return ParsedFilename {
            track_num: None,
            artist: None,
            title: Some(stem.to_string()),
        };
    }

    let first_two = &stem[..2];

    // Check for disc-track format: D-NN
    let (track_num_str, remainder) = if bytes.len() >= 5
        && (bytes[1] == b'-' || bytes[1] == b'.')
        && bytes[0].is_ascii_digit()
        && bytes[2].is_ascii_digit()
        && bytes[3].is_ascii_digit()
    {
        let disc_track = &stem[..4];
        (Some(disc_track.to_string()), stem[4..].trim_start())
    } else {
        try_parse_track_number(first_two, stem)
    };

    if let Some(sep_pos) = remainder.find(" - ") {
        let artist = remainder[..sep_pos].trim();
        let title = remainder[sep_pos + 3..].trim();
        ParsedFilename {
            track_num: track_num_str,
            artist: if artist.is_empty() {
                None
            } else {
                Some(artist.to_string())
            },
            title: if title.is_empty() {
                None
            } else {
                Some(title.to_string())
            },
        }
    } else if let Some(title) = remainder.strip_prefix(". ") {
        // Acceptable alternate: "NN. Title" — only match leading ". " to avoid
        // splitting on interior dots (e.g. "feat. Someone").
        let title = title.trim();
        ParsedFilename {
            track_num: track_num_str,
            artist: None,
            title: if title.is_empty() {
                None
            } else {
                Some(title.to_string())
            },
        }
    } else {
        ParsedFilename {
            track_num: track_num_str,
            artist: None,
            title: Some(remainder.to_string()),
        }
    }
}

pub(crate) fn strip_track_separator(rest: &str) -> Option<&str> {
    if let Some(title) = rest.strip_prefix(" - ") {
        return Some(title);
    }
    if let Some(title) = rest.strip_prefix(' ') {
        return Some(title);
    }
    if rest.starts_with("- ") {
        return Some(rest.trim_start_matches(['-', ' ']));
    }
    if let Some(title) = rest.strip_prefix(". ") {
        return Some(title);
    }
    if let Some(title) = rest.strip_prefix('.') {
        return Some(title);
    }
    // dash without space: only if NOT followed by a digit
    if rest.len() >= 2 && rest.as_bytes()[0] == b'-' && !rest.as_bytes()[1].is_ascii_digit() {
        return Some(&rest[1..]);
    }
    None
}

pub(crate) fn try_parse_track_number<'a>(
    first_two: &str,
    stem: &'a str,
) -> (Option<String>, &'a str) {
    let bytes = stem.as_bytes();
    if first_two.chars().all(|c| c.is_ascii_digit()) {
        if bytes.len() > 2
            && bytes[2].is_ascii_digit()
            && (bytes.len() == 3 || bytes[3] == b' ' || bytes[3] == b'-' || bytes[3] == b'.')
        {
            let num = stem[..3].to_string();
            let rest = &stem[3..];
            return match strip_track_separator(rest) {
                Some(remainder) => (Some(num), remainder),
                None => (None, stem),
            };
        }

        let num = first_two.to_string();
        let rest = &stem[2..];
        match strip_track_separator(rest) {
            Some(remainder) => (Some(num), remainder),
            None => (None, stem),
        }
    } else {
        (None, stem)
    }
}

pub(crate) fn parse_loose_filename(stem: &str) -> ParsedFilename {
    if let Some(sep_pos) = stem.find(" - ") {
        let artist = stem[..sep_pos].trim();
        let title = stem[sep_pos + 3..].trim();
        ParsedFilename {
            track_num: None,
            artist: if artist.is_empty() {
                None
            } else {
                Some(artist.to_string())
            },
            title: if title.is_empty() {
                None
            } else {
                Some(title.to_string())
            },
        }
    } else {
        ParsedFilename {
            track_num: None,
            artist: None,
            title: Some(stem.to_string()),
        }
    }
}

// ---------------------------------------------------------------------------
// Detected issue
// ---------------------------------------------------------------------------
