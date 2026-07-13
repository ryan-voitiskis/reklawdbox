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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_album_track_with_year() {
        let p = Path::new("/music/Artist/Album Name (2024)/01 Artist - Track.flac");
        assert_eq!(
            classify_track_context(p, &HashSet::new()),
            AuditContext::AlbumTrack
        );
    }

    #[test]
    fn classify_album_track_with_tech_specs_and_year() {
        let p = Path::new("/music/Artist/Album [FLAC] (2024)/01 Artist - Track.flac");
        assert_eq!(
            classify_track_context(p, &HashSet::new()),
            AuditContext::AlbumTrack
        );
    }

    #[test]
    fn classify_loose_track_in_unnamed_dir() {
        let p = Path::new("/music/play/Artist - Track.wav");
        assert_eq!(
            classify_track_context(p, &HashSet::new()),
            AuditContext::LooseTrack
        );
    }

    #[test]
    fn classify_loose_track_no_year() {
        let p = Path::new("/music/Artist/SomeDir/Artist - Track.flac");
        assert_eq!(
            classify_track_context(p, &HashSet::new()),
            AuditContext::LooseTrack
        );
    }

    #[test]
    fn classify_disc_subdir() {
        let p = Path::new("/music/Artist/Album (2020)/CD1/01 Artist - Track.flac");
        assert_eq!(
            classify_track_context(p, &HashSet::new()),
            AuditContext::AlbumTrack
        );
    }

    // -- has_year_suffix --

    #[test]
    fn year_suffix_present() {
        assert!(has_year_suffix("Album Name (2024)"));
        assert!(has_year_suffix("Album (1999)"));
    }

    #[test]
    fn year_suffix_absent() {
        assert!(!has_year_suffix("Album Name"));
        assert!(!has_year_suffix("Album (Deluxe)"));
        assert!(!has_year_suffix("(20)"));
    }

    #[test]
    fn year_suffix_unicode_boundaries_are_safe() {
        assert!(!has_year_suffix("Album (日本2024)"));
        assert!(!has_year_suffix("Album (🎵2024)"));
        assert!(has_year_suffix("日本語のアルバム (2024)"));
        assert!(has_year_suffix("Album (2024 日本盤)"));
    }

    // -- has_year_range --

    #[test]
    fn year_range_present() {
        assert!(has_year_range("The Studio Album Collection 1977-1992"));
        assert!(has_year_range("Live 1992-2014"));
        assert!(has_year_range("Anthology 2000\u{2013}2020")); // en-dash
    }

    #[test]
    fn year_range_absent() {
        assert!(!has_year_range("Album Name"));
        assert!(!has_year_range("CCCP Edits 4"));
        assert!(!has_year_range("Album 123-456")); // not valid years
        assert!(!has_year_range("Album 1899-2100")); // out of range
    }

    // -- has_bare_year --

    #[test]
    fn bare_year_present() {
        assert!(has_bare_year(
            "Live at Alexandra Palace - London 8th and 9th May 2019"
        ));
        assert!(has_bare_year("FM Broadcast August 1996"));
        assert!(has_bare_year("Live in Tokyo - 1st December 2013"));
    }

    #[test]
    fn bare_year_absent() {
        assert!(!has_bare_year("Album Name"));
        assert!(!has_bare_year("CCCP Edits 4"));
        assert!(!has_bare_year("Return to Nothing"));
        assert!(!has_bare_year("Fever (Limited Edition)"));
    }

    // -- parse_filename --

    #[test]
    fn parse_album_canonical() {
        let p = Path::new("/music/Artist/Album (2024)/01 Some Artist - Track Title.flac");
        let parsed = parse_filename(p, &AuditContext::AlbumTrack);
        assert_eq!(parsed.track_num.as_deref(), Some("01"));
        assert_eq!(parsed.artist.as_deref(), Some("Some Artist"));
        assert_eq!(parsed.title.as_deref(), Some("Track Title"));
    }

    #[test]
    fn parse_album_dot_format() {
        let p = Path::new("/music/Artist/Album (2024)/08. Tune Out.flac");
        let parsed = parse_filename(p, &AuditContext::AlbumTrack);
        assert_eq!(parsed.track_num.as_deref(), Some("08"));
        assert_eq!(parsed.artist, None);
        assert_eq!(parsed.title.as_deref(), Some("Tune Out"));
    }

    #[test]
    fn parse_loose_canonical() {
        let p = Path::new("/music/play/Burial - Archangel.wav");
        let parsed = parse_filename(p, &AuditContext::LooseTrack);
        assert_eq!(parsed.track_num, None);
        assert_eq!(parsed.artist.as_deref(), Some("Burial"));
        assert_eq!(parsed.title.as_deref(), Some("Archangel"));
    }

    #[test]
    fn parse_loose_no_separator() {
        let p = Path::new("/music/play/JustATitle.wav");
        let parsed = parse_filename(p, &AuditContext::LooseTrack);
        assert_eq!(parsed.artist, None);
        assert_eq!(parsed.title.as_deref(), Some("JustATitle"));
    }

    #[test]
    fn parse_title_with_hyphen() {
        let p = Path::new("/music/play/Artist - Title - Subtitle.flac");
        let parsed = parse_filename(p, &AuditContext::LooseTrack);
        assert_eq!(parsed.artist.as_deref(), Some("Artist"));
        assert_eq!(parsed.title.as_deref(), Some("Title - Subtitle"));
    }

    // -- check_tags --

    #[test]
    fn normalize_strips_tech_specs() {
        // Existing cases
        assert_eq!(normalize_dir_name("Album [FLAC] (2024)"), "Album (2024)");
        assert_eq!(normalize_dir_name("Album [WAV] 24-96"), "Album");
        // Issue #15: fractional kHz suffix left `.1` fragment
        assert_eq!(
            normalize_dir_name("Good Lies(Electronic) [2023] 24-44.1"),
            "Good Lies(Electronic) [2023]"
        );
        // Issue #15: bare format + units left `(-44.1kHz)` fragment
        assert_eq!(normalize_dir_name("FLAC (16bit-44.1kHz)"), "");
        // Bare format name without brackets
        assert_eq!(normalize_dir_name("Album FLAC"), "Album");
        // Standalone bit-depth with space
        assert_eq!(normalize_dir_name("Album 24 bit"), "Album");
        // Sample rate with kHz unit
        assert_eq!(normalize_dir_name("Album 16-48kHz"), "Album");
    }

    #[test]
    fn normalize_no_false_positives() {
        // Years in parens must NOT be stripped
        assert_eq!(normalize_dir_name("Album Name (2024)"), "Album Name (2024)");
        assert_eq!(normalize_dir_name("Album (2016)"), "Album (2016)");
        // Non-tech-spec parenthesized text must NOT be stripped
        assert_eq!(normalize_dir_name("Album (Deluxe)"), "Album (Deluxe)");
        // Bare number that happens to be 16/24/32 but not followed by bit/- pattern
        assert_eq!(normalize_dir_name("Track 24"), "Track 24");
        assert_eq!(normalize_dir_name("Studio 32"), "Studio 32");
        // Two-digit numbers that are NOT 16/24/32 followed by dash should not match
        assert_eq!(normalize_dir_name("Album 20-20"), "Album 20-20");
        // "24" embedded in longer number should not match
        assert_eq!(normalize_dir_name("Track 2400"), "Track 2400");
        assert_eq!(normalize_dir_name("Track 124"), "Track 124");
        // Artist/album name containing "wav" as substring should NOT match
        assert_eq!(
            normalize_dir_name("Brainwave Sessions"),
            "Brainwave Sessions"
        );
        assert_eq!(
            normalize_dir_name("New Wave Compilation"),
            "New Wave Compilation"
        );
    }

    // -- IssueType round-trip --

    #[test]
    fn parse_album_multibyte_utf8_no_panic() {
        // 3-byte char at start: should not panic
        let p = Path::new("/music/Artist/Album (2024)/€1 Artist - Title.flac");
        let parsed = parse_filename(p, &AuditContext::AlbumTrack);
        // Should return something (possibly no track_num) but must NOT panic
        assert!(parsed.title.is_some());
    }

    // Finding 3: ARTIST_IN_TITLE new_title is correct with unicode

    #[test]
    fn parse_album_nn_dash_title() {
        let p = Path::new("/music/Artist/Album (2024)/05 - Invisible Dance.flac");
        let parsed = parse_filename(p, &AuditContext::AlbumTrack);
        assert_eq!(parsed.track_num.as_deref(), Some("05"));
        assert_eq!(parsed.title.as_deref(), Some("Invisible Dance"));
        assert_eq!(parsed.artist, None);
    }

    // Finding 10: Missing-space format is bad filename

    #[test]
    fn parse_album_missing_space_is_bad() {
        let p = Path::new("/music/Artist/Album (2024)/01Artist - Title.flac");
        let parsed = parse_filename(p, &AuditContext::AlbumTrack);
        // Should NOT extract track number (no valid separator)
        assert_eq!(parsed.track_num, None);
    }

    // Finding 6: Directory checks use album dir for disc subdirs

    #[test]
    fn year_suffix_compound_years() {
        assert!(has_year_suffix("Album (1969, 2004)"));
        assert!(has_year_suffix("Album (2017, Label - Cat)"));
        assert!(has_year_suffix("Album (2020 Remaster)"));
    }

    #[test]
    fn year_suffix_range_validation() {
        assert!(has_year_suffix("Album (1900)"));
        assert!(has_year_suffix("Album (2099)"));
        assert!(!has_year_suffix("Album (1899)"));
        assert!(!has_year_suffix("Album (2100)"));
        assert!(!has_year_suffix("Album (0001)"));
    }

    #[test]
    fn classify_album_with_catalog_suffix() {
        let p = Path::new("/music/Artist/Album (2017, Label - Cat)/01 Artist - Track.flac");
        assert_eq!(
            classify_track_context(p, &HashSet::new()),
            AuditContext::AlbumTrack
        );
    }

    // -- detect_album_dirs --

    #[test]
    fn detect_album_dirs_two_numbered() {
        let paths = vec![
            std::path::PathBuf::from("/music/Artist/Mix/01 Track.flac"),
            std::path::PathBuf::from("/music/Artist/Mix/02 Track.flac"),
        ];
        let dirs = detect_album_dirs(&paths);
        assert!(dirs.contains(Path::new("/music/Artist/Mix")));
    }

    #[test]
    fn detect_album_dirs_one_numbered_not_detected() {
        let paths = vec![
            std::path::PathBuf::from("/music/Artist/Mix/01 Track.flac"),
            std::path::PathBuf::from("/music/Artist/Mix/Intro.flac"),
        ];
        let dirs = detect_album_dirs(&paths);
        assert!(!dirs.contains(Path::new("/music/Artist/Mix")));
    }

    #[test]
    fn detect_album_dirs_zero_numbered() {
        let paths = vec![
            std::path::PathBuf::from("/music/Artist/Mix/Intro.flac"),
            std::path::PathBuf::from("/music/Artist/Mix/Outro.flac"),
        ];
        let dirs = detect_album_dirs(&paths);
        assert!(dirs.is_empty());
    }

    #[test]
    fn classify_with_album_dirs_no_year() {
        let album_dirs: HashSet<std::path::PathBuf> =
            [std::path::PathBuf::from("/music/Artist/Giegling Mix")].into();
        let p = Path::new("/music/Artist/Giegling Mix/01 Track.flac");
        assert_eq!(
            classify_track_context(p, &album_dirs),
            AuditContext::AlbumTrack
        );
    }

    #[test]
    fn classify_disc_subdir_with_album_dirs() {
        let album_dirs: HashSet<std::path::PathBuf> =
            [std::path::PathBuf::from("/music/Artist/Mix")].into();
        let p = Path::new("/music/Artist/Mix/CD1/01 Track.flac");
        assert_eq!(
            classify_track_context(p, &album_dirs),
            AuditContext::AlbumTrack
        );
    }

    // -- TECH_SPEC_RE: hi-res --

    #[test]
    fn normalize_strips_hi_res() {
        assert_eq!(normalize_dir_name("Album [Hi-Res] (2024)"), "Album (2024)");
        assert_eq!(normalize_dir_name("Album [HiRes]"), "Album");
    }

    // -- D.NN disc-dot parsing --

    #[test]
    fn parse_album_disc_dot_format() {
        let p = Path::new("/music/Artist/Album (2024)/1.01 Artist - Track.flac");
        let parsed = parse_filename(p, &AuditContext::AlbumTrack);
        assert_eq!(parsed.track_num.as_deref(), Some("1.01"));
        assert_eq!(parsed.artist.as_deref(), Some("Artist"));
        assert_eq!(parsed.title.as_deref(), Some("Track"));
    }

    // -- 3-digit track numbers --

    #[test]
    fn parse_album_three_digit_track() {
        let p = Path::new("/music/Artist/Album (2024)/100 Artist - Track.flac");
        let parsed = parse_filename(p, &AuditContext::AlbumTrack);
        assert_eq!(parsed.track_num.as_deref(), Some("100"));
        assert_eq!(parsed.artist.as_deref(), Some("Artist"));
        assert_eq!(parsed.title.as_deref(), Some("Track"));
    }

    #[test]
    fn parse_album_two_digit_no_regression() {
        let p = Path::new("/music/Artist/Album (2024)/01 Artist - Track.flac");
        let parsed = parse_filename(p, &AuditContext::AlbumTrack);
        assert_eq!(parsed.track_num.as_deref(), Some("01"));
        assert_eq!(parsed.artist.as_deref(), Some("Artist"));
        assert_eq!(parsed.title.as_deref(), Some("Track"));
    }

    #[test]
    fn parse_album_three_digit_dot_format() {
        let p = Path::new("/music/Artist/Album (2024)/100. Track Title.flac");
        let parsed = parse_filename(p, &AuditContext::AlbumTrack);
        assert_eq!(parsed.track_num.as_deref(), Some("100"));
        assert_eq!(parsed.title.as_deref(), Some("Track Title"));
    }

    // -- Drift normalization --

    #[test]
    fn parse_album_nn_dash_no_space() {
        let p = Path::new("/music/Artist/Album (2024)/01-Dreamin.flac");
        let parsed = parse_filename(p, &AuditContext::AlbumTrack);
        assert_eq!(parsed.track_num.as_deref(), Some("01"));
        assert_eq!(parsed.title.as_deref(), Some("Dreamin"));
    }

    #[test]
    fn parse_album_nn_dash_no_space_with_artist() {
        let p = Path::new("/music/Artist/Album (2024)/08-Snoop Doggy Dogg - Gold Rush.flac");
        let parsed = parse_filename(p, &AuditContext::AlbumTrack);
        assert_eq!(parsed.track_num.as_deref(), Some("08"));
        assert_eq!(parsed.artist.as_deref(), Some("Snoop Doggy Dogg"));
        assert_eq!(parsed.title.as_deref(), Some("Gold Rush"));
    }

    #[test]
    fn parse_album_three_digit_dash_no_space() {
        let p = Path::new("/music/Artist/Album (2024)/100-Track.flac");
        let parsed = parse_filename(p, &AuditContext::AlbumTrack);
        assert_eq!(parsed.track_num.as_deref(), Some("100"));
        assert_eq!(parsed.title.as_deref(), Some("Track"));
    }

    #[test]
    fn parse_album_nn_dash_digit_not_parsed_as_track() {
        // "01-02" — dash followed by digit should NOT be parsed as NN-Title
        let p = Path::new("/music/Artist/Album (2024)/01-02 Artist - Title.flac");
        let parsed = parse_filename(p, &AuditContext::AlbumTrack);
        // Neither disc-track (bytes[1] is '1' not '-') nor NN-Title (digit after dash)
        assert_eq!(parsed.track_num, None);
    }

    // -- Disc subdir with album_dirs pre-pass --

    #[test]
    fn classify_disc_subdir_detected_by_prepass() {
        // CD1 is in album_dirs (pre-pass detected it), grandparent has no year suffix
        let album_dirs: HashSet<std::path::PathBuf> =
            [std::path::PathBuf::from("/music/Artist/Live 1992-2014/CD1")].into();
        let p = Path::new("/music/Artist/Live 1992-2014/CD1/01 Artist - Track.flac");
        assert_eq!(
            classify_track_context(p, &album_dirs),
            AuditContext::AlbumTrack
        );
    }

    // -- Dot-space prefix stripping --

    #[test]
    fn parse_album_dot_format_with_artist() {
        // "01. Artist - Title" — the ". " prefix must not leak into artist
        let p = Path::new("/music/Artist/Album (2024)/01. Roza Terenzi - Loose.flac");
        let parsed = parse_filename(p, &AuditContext::AlbumTrack);
        assert_eq!(parsed.track_num.as_deref(), Some("01"));
        assert_eq!(parsed.artist.as_deref(), Some("Roza Terenzi"));
        assert_eq!(parsed.title.as_deref(), Some("Loose"));
    }

    #[test]
    fn parse_album_feat_dot_no_false_split() {
        // "feat." interior dot must not split the title
        let p = Path::new("/music/Artist/Album (2024)/03 Artist feat. Someone.flac");
        let parsed = parse_filename(p, &AuditContext::AlbumTrack);
        assert_eq!(parsed.track_num.as_deref(), Some("03"));
        assert_eq!(parsed.title.as_deref(), Some("Artist feat. Someone"));
    }

    // -- Original Mix symmetric stripping --

    #[test]
    fn disc_subdir_rejects_false_positives() {
        assert!(!is_disc_subdir("Disco Dreams Unlimited (2018)"));
        assert!(!is_disc_subdir("Discovering Infinity"));
        assert!(!is_disc_subdir("CD"));
        assert!(!is_disc_subdir("Disc"));
        assert!(!is_disc_subdir("Disconnected"));
    }

    #[test]
    fn disc_subdir_accepts_real_disc_dirs() {
        assert!(is_disc_subdir("CD1"));
        assert!(is_disc_subdir("CD 2"));
        assert!(is_disc_subdir("CD10"));
        assert!(is_disc_subdir("Disc 1"));
        assert!(is_disc_subdir("disc2"));
        assert!(is_disc_subdir("Disk 3"));
    }

    // -- ancestor_has_year --

    #[test]
    fn ancestor_year_suppresses_nested_subdir() {
        let p = Path::new("/music/Artist/Album (2021)/bonus-tracks/01 track.flac");
        assert!(ancestor_has_year(p));
    }

    #[test]
    fn ancestor_year_absent_when_no_year_above() {
        let p = Path::new("/music/Artist/SomeDir/nested/01 track.flac");
        assert!(!ancestor_has_year(p));
    }

    // -- leaf-dir filter in detect_album_dirs --

    #[test]
    fn leaf_dir_filter_excludes_parent_of_album_dir() {
        use std::path::PathBuf;
        let paths = vec![
            // parent dir has loose numbered files
            PathBuf::from("/music/lossy/01 Track A.flac"),
            PathBuf::from("/music/lossy/02 Track B.flac"),
            // child subdir also has numbered files
            PathBuf::from("/music/lossy/Artist/01 Track X.flac"),
            PathBuf::from("/music/lossy/Artist/02 Track Y.flac"),
        ];
        let album_dirs = detect_album_dirs(&paths);
        // /music/lossy/Artist is a leaf → included
        assert!(album_dirs.contains(&PathBuf::from("/music/lossy/Artist")));
        // /music/lossy is parent of a child album dir → excluded
        assert!(!album_dirs.contains(&PathBuf::from("/music/lossy")));
    }

    #[test]
    fn leaf_dir_filter_keeps_standalone_album_dir() {
        use std::path::PathBuf;
        let paths = vec![
            PathBuf::from("/music/Artist/Album/01 Track.flac"),
            PathBuf::from("/music/Artist/Album/02 Track.flac"),
        ];
        let album_dirs = detect_album_dirs(&paths);
        assert!(album_dirs.contains(&PathBuf::from("/music/Artist/Album")));
    }
}
