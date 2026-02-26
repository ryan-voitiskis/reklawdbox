//! Audit engine: convention checks, filename parsing, scan orchestration.
//!
//! Pure convention-check functions take a path + `FileReadResult` + context and
//! return detected issues. The scan operation walks the filesystem, reads tags,
//! applies checks, and persists results to SQLite.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::Path;

use rusqlite::Connection;
use serde::Serialize;
use unicode_casefold::UnicodeCaseFold;

use crate::store;
use crate::tags::{self, FileReadResult};

// ---------------------------------------------------------------------------
// Issue types & safety tiers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, strum::EnumString, strum::EnumIter)]
pub enum IssueType {
    #[strum(serialize = "EMPTY_ARTIST")]
    EmptyArtist,
    #[strum(serialize = "EMPTY_TITLE")]
    EmptyTitle,
    #[strum(serialize = "MISSING_TRACK_NUM")]
    MissingTrackNum,
    #[strum(serialize = "MISSING_ALBUM")]
    MissingAlbum,
    #[strum(serialize = "MISSING_YEAR")]
    MissingYear,
    #[strum(serialize = "ARTIST_IN_TITLE")]
    ArtistInTitle,
    #[strum(serialize = "WAV_TAG3_MISSING")]
    WavTag3Missing,
    #[strum(serialize = "WAV_TAG_DRIFT")]
    WavTagDrift,
    #[strum(serialize = "GENRE_SET")]
    GenreSet,
    #[strum(serialize = "NO_TAGS")]
    NoTags,
    #[strum(serialize = "BAD_FILENAME")]
    BadFilename,
    #[strum(serialize = "ORIGINAL_MIX_SUFFIX")]
    OriginalMixSuffix,
    #[strum(serialize = "TECH_SPECS_IN_DIR")]
    TechSpecsInDir,
    #[strum(serialize = "MISSING_YEAR_IN_DIR")]
    MissingYearInDir,
    #[strum(serialize = "FILENAME_TAG_DRIFT")]
    FilenameTagDrift,
}

impl IssueType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::EmptyArtist => "EMPTY_ARTIST",
            Self::EmptyTitle => "EMPTY_TITLE",
            Self::MissingTrackNum => "MISSING_TRACK_NUM",
            Self::MissingAlbum => "MISSING_ALBUM",
            Self::MissingYear => "MISSING_YEAR",
            Self::ArtistInTitle => "ARTIST_IN_TITLE",
            Self::WavTag3Missing => "WAV_TAG3_MISSING",
            Self::WavTagDrift => "WAV_TAG_DRIFT",
            Self::GenreSet => "GENRE_SET",
            Self::NoTags => "NO_TAGS",
            Self::BadFilename => "BAD_FILENAME",
            Self::OriginalMixSuffix => "ORIGINAL_MIX_SUFFIX",
            Self::TechSpecsInDir => "TECH_SPECS_IN_DIR",
            Self::MissingYearInDir => "MISSING_YEAR_IN_DIR",
            Self::FilenameTagDrift => "FILENAME_TAG_DRIFT",
        }
    }

    pub fn safety_tier(&self) -> SafetyTier {
        match self {
            Self::ArtistInTitle | Self::WavTag3Missing | Self::WavTagDrift => SafetyTier::Safe,
            Self::OriginalMixSuffix | Self::TechSpecsInDir => SafetyTier::RenameSafe,
            Self::EmptyArtist
            | Self::EmptyTitle
            | Self::MissingTrackNum
            | Self::MissingAlbum
            | Self::MissingYear
            | Self::GenreSet
            | Self::NoTags
            | Self::BadFilename
            | Self::MissingYearInDir
            | Self::FilenameTagDrift => SafetyTier::Review,
        }
    }
}

impl fmt::Display for IssueType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SafetyTier {
    Safe,
    RenameSafe,
    Review,
}

// ---------------------------------------------------------------------------
// Audit status & resolution
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AuditStatus {
    Open,
    Resolved,
    Accepted,
    Deferred,
}

impl AuditStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Resolved => "resolved",
            Self::Accepted => "accepted",
            Self::Deferred => "deferred",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "open" => Some(Self::Open),
            "resolved" => Some(Self::Resolved),
            "accepted" => Some(Self::Accepted),
            "deferred" => Some(Self::Deferred),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Resolution {
    AcceptedAsIs,
    WontFix,
    Deferred,
    Fixed,
}

impl Resolution {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AcceptedAsIs => "accepted_as_is",
            Self::WontFix => "wont_fix",
            Self::Deferred => "deferred",
            Self::Fixed => "fixed",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "accepted_as_is" => Some(Self::AcceptedAsIs),
            "wont_fix" => Some(Self::WontFix),
            "deferred" => Some(Self::Deferred),
            "fixed" => Some(Self::Fixed),
            _ => None,
        }
    }

    pub fn status(&self) -> AuditStatus {
        match self {
            Self::AcceptedAsIs | Self::WontFix => AuditStatus::Accepted,
            Self::Deferred => AuditStatus::Deferred,
            Self::Fixed => AuditStatus::Resolved,
        }
    }
}

// ---------------------------------------------------------------------------
// Track context
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditContext {
    AlbumTrack,
    LooseTrack,
}

/// Check if a directory name represents a disc subdirectory (CD1, Disc 1, etc.).
fn is_disc_subdir(name: &str) -> bool {
    name.starts_with("CD") || name.starts_with("Disc") || name.starts_with("disc")
}

/// Lowercase patterns for tech specs in directory names.
/// Matching is always done against the lowercased input.
const TECH_SPEC_PATTERNS: &[&str] = &[
    "[flac]", "[wav]", "[mp3]", "[aiff]", "[aac]",
    "24-96", "24-48", "24-44", "16-44", "16-48",
    "24bit", "16bit",
];

/// Strip tech-spec brackets and bitrate specs from a directory name for
/// pattern matching. Matching is case-insensitive but non-pattern text
/// preserves its original casing.
fn normalize_dir_name(name: &str) -> String {
    let mut result = name.to_string();
    for pat in TECH_SPEC_PATTERNS {
        // Find the pattern in the lowercased string, then remove the
        // corresponding byte range from the original to preserve casing.
        loop {
            let lower = result.to_ascii_lowercase();
            if let Some(pos) = lower.find(pat) {
                result.replace_range(pos..pos + pat.len(), "");
            } else {
                break;
            }
        }
    }
    // Collapse multiple spaces into one
    while result.contains("  ") {
        result = result.replace("  ", " ");
    }
    result.trim().to_string()
}

/// Check if a directory name has a year suffix like `(2024)`.
fn has_year_suffix(name: &str) -> bool {
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
        inside.len() == 4 && inside.parse::<u16>().is_ok()
    } else {
        false
    }
}

pub fn classify_track_context(path: &Path) -> AuditContext {
    let parent = match path.parent() {
        Some(p) => p,
        None => return AuditContext::LooseTrack,
    };

    let dir_name = match parent.file_name().and_then(|n| n.to_str()) {
        Some(n) => n,
        None => return AuditContext::LooseTrack,
    };

    // Check for disc subdirectories (CD1, CD2, Disc 1, etc.)
    let effective_dir_name = if is_disc_subdir(dir_name) {
        // Go up one more level for the album dir
        match parent.parent().and_then(|p| p.file_name()).and_then(|n| n.to_str()) {
            Some(n) => n,
            None => return AuditContext::LooseTrack,
        }
    } else {
        dir_name
    };

    // Check if in a play/ directory
    let lower = effective_dir_name.to_lowercase();
    if lower == "play" || lower.starts_with("play/") {
        return AuditContext::LooseTrack;
    }

    let normalized = normalize_dir_name(effective_dir_name);
    if has_year_suffix(&normalized) {
        AuditContext::AlbumTrack
    } else {
        AuditContext::LooseTrack
    }
}

/// Get the effective album directory name, climbing past disc subdirectories.
fn effective_album_dir_name(path: &Path) -> Option<(&Path, &str)> {
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

fn parse_album_filename(stem: &str) -> ParsedFilename {
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
        && bytes[1] == b'-'
        && bytes[0].is_ascii_digit()
        && bytes[2].is_ascii_digit()
        && bytes[3].is_ascii_digit()
    {
        let disc_track = &stem[..4];
        (Some(disc_track.to_string()), stem[4..].trim_start())
    } else {
        try_parse_track_number(first_two, stem)
    };

    // Try to split remainder on " - " for artist - title
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
    } else if let Some(sep_pos) = remainder.find(". ") {
        // Acceptable alternate: "NN. Title"
        let title = remainder[sep_pos + 2..].trim();
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

fn try_parse_track_number<'a>(first_two: &str, stem: &'a str) -> (Option<String>, &'a str) {
    if first_two.chars().all(|c| c.is_ascii_digit()) {
        let num = first_two.to_string();
        let rest = &stem[2..];
        let remainder = if rest.starts_with(" - ") {
            // "NN - Title" alternate format
            &rest[3..]
        } else if rest.starts_with(' ') {
            // "NN Artist - Title" canonical format (skip the space)
            &rest[1..]
        } else if rest.starts_with("- ") {
            rest.trim_start_matches(|c: char| c == '-' || c == ' ')
        } else if rest.starts_with(". ") || rest.starts_with('.') {
            // "NN. Title" alternate format — keep the dot+rest so
            // parse_album_filename's ". " branch can split it.
            rest
        } else {
            // No valid separator after track number — not a valid track-numbered filename
            return (None, stem);
        };
        (Some(num), remainder)
    } else {
        (None, stem)
    }
}

fn parse_loose_filename(stem: &str) -> ParsedFilename {
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

#[derive(Debug, Clone)]
pub struct DetectedIssue {
    pub issue_type: IssueType,
    pub detail: Option<String>,
}

// ---------------------------------------------------------------------------
// Convention checks — pure functions
// ---------------------------------------------------------------------------

/// Get the effective tag map (primary layer) from a FileReadResult.
fn get_tag_value(result: &FileReadResult, field: &str) -> Option<String> {
    match result {
        FileReadResult::Single { tags, .. } => tags.get(field).and_then(|v| v.clone()),
        FileReadResult::Wav { id3v2, .. } => id3v2.get(field).and_then(|v| v.clone()),
        FileReadResult::Error { .. } => None,
    }
}

fn tag_is_empty(result: &FileReadResult, field: &str) -> bool {
    get_tag_value(result, field)
        .as_ref()
        .is_none_or(|v| v.trim().is_empty())
}

fn all_tags_empty(result: &FileReadResult) -> bool {
    tags::ALL_FIELDS.iter().all(|&f| tag_is_empty(result, f))
}

fn is_wav(result: &FileReadResult) -> bool {
    matches!(result, FileReadResult::Wav { .. })
}

fn casefold_text(s: &str) -> String {
    s.case_fold().collect()
}

pub fn check_tags(
    path: &Path,
    read_result: &FileReadResult,
    context: &AuditContext,
    skip: &HashSet<IssueType>,
) -> Vec<DetectedIssue> {
    let _ = path; // path reserved for future use
    let mut issues = Vec::new();

    // NO_TAGS — check first; if all empty, skip other tag checks
    if !skip.contains(&IssueType::NoTags) && all_tags_empty(read_result) {
        issues.push(DetectedIssue {
            issue_type: IssueType::NoTags,
            detail: None,
        });
        return issues;
    }

    // EMPTY_ARTIST
    if !skip.contains(&IssueType::EmptyArtist) && tag_is_empty(read_result, "artist") {
        issues.push(DetectedIssue {
            issue_type: IssueType::EmptyArtist,
            detail: None,
        });
    }

    // EMPTY_TITLE
    if !skip.contains(&IssueType::EmptyTitle) && tag_is_empty(read_result, "title") {
        issues.push(DetectedIssue {
            issue_type: IssueType::EmptyTitle,
            detail: None,
        });
    }

    // Album-track-only checks
    if *context == AuditContext::AlbumTrack {
        if !skip.contains(&IssueType::MissingTrackNum) && tag_is_empty(read_result, "track") {
            issues.push(DetectedIssue {
                issue_type: IssueType::MissingTrackNum,
                detail: None,
            });
        }

        if !skip.contains(&IssueType::MissingAlbum) && tag_is_empty(read_result, "album") {
            issues.push(DetectedIssue {
                issue_type: IssueType::MissingAlbum,
                detail: None,
            });
        }

        if !skip.contains(&IssueType::MissingYear)
            && tag_is_empty(read_result, "year")
            && tag_is_empty(read_result, "date")
        {
            issues.push(DetectedIssue {
                issue_type: IssueType::MissingYear,
                detail: None,
            });
        }
    }

    // ARTIST_IN_TITLE
    if !skip.contains(&IssueType::ArtistInTitle) {
        if let (Some(artist), Some(title)) = (
            get_tag_value(read_result, "artist"),
            get_tag_value(read_result, "title"),
        ) {
            let artist_trimmed = artist.trim();
            if !artist_trimmed.is_empty() {
                let artist_folded = casefold_text(artist_trimmed);
                for (sep_pos, _) in title.match_indices(" - ") {
                    let candidate_artist = &title[..sep_pos];
                    if casefold_text(candidate_artist) == artist_folded {
                        let clean_title = title[sep_pos + 3..].to_string();
                        issues.push(DetectedIssue {
                            issue_type: IssueType::ArtistInTitle,
                            detail: Some(
                                serde_json::json!({
                                    "artist": artist_trimmed,
                                    "old_title": title,
                                    "new_title": clean_title,
                                })
                                .to_string(),
                            ),
                        });
                        break;
                    }
                }
            }
        }
    }

    // WAV-specific checks
    if is_wav(read_result) {
        if let FileReadResult::Wav {
            tag3_missing,
            id3v2,
            riff_info,
            ..
        } = read_result
        {
            // WAV_TAG3_MISSING
            if !skip.contains(&IssueType::WavTag3Missing) && !tag3_missing.is_empty() {
                issues.push(DetectedIssue {
                    issue_type: IssueType::WavTag3Missing,
                    detail: Some(
                        serde_json::json!({ "fields": tag3_missing }).to_string(),
                    ),
                });
            }

            // WAV_TAG_DRIFT
            if !skip.contains(&IssueType::WavTagDrift) {
                let mut drifted = Vec::new();
                for field in &["artist", "title", "album", "genre", "year", "comment"] {
                    let v2 = id3v2.get(*field).and_then(|v| v.as_deref()).map(|s| s.trim());
                    let ri = riff_info.get(*field).and_then(|v| v.as_deref()).map(|s| s.trim());
                    if let (Some(v2_val), Some(ri_val)) = (v2, ri) {
                        if v2_val != ri_val {
                            drifted.push(serde_json::json!({
                                "field": field,
                                "id3v2": v2_val,
                                "riff_info": ri_val,
                            }));
                        }
                    }
                }
                if !drifted.is_empty() {
                    issues.push(DetectedIssue {
                        issue_type: IssueType::WavTagDrift,
                        detail: Some(serde_json::json!({ "drifted": drifted }).to_string()),
                    });
                }
            }
        }
    }

    // GENRE_SET
    if !skip.contains(&IssueType::GenreSet) && !tag_is_empty(read_result, "genre") {
        let genre_val = get_tag_value(read_result, "genre").unwrap();
        issues.push(DetectedIssue {
            issue_type: IssueType::GenreSet,
            detail: Some(serde_json::json!({ "genre": genre_val }).to_string()),
        });
    }

    issues
}

pub fn check_filename(
    path: &Path,
    read_result: &FileReadResult,
    context: &AuditContext,
    skip: &HashSet<IssueType>,
) -> Vec<DetectedIssue> {
    let mut issues = Vec::new();

    let filename = match path.file_name().and_then(|n| n.to_str()) {
        Some(n) => n,
        None => return issues,
    };

    // ORIGINAL_MIX_SUFFIX
    if !skip.contains(&IssueType::OriginalMixSuffix) && filename.contains("(Original Mix)") {
        let new_name = filename.replace(" (Original Mix)", "").replace("(Original Mix)", "");
        issues.push(DetectedIssue {
            issue_type: IssueType::OriginalMixSuffix,
            detail: Some(
                serde_json::json!({
                    "old_filename": filename,
                    "new_filename": new_name.trim(),
                })
                .to_string(),
            ),
        });
    }

    // TECH_SPECS_IN_DIR
    if !skip.contains(&IssueType::TechSpecsInDir) {
        if let Some((_, dir_name)) = effective_album_dir_name(path) {
            let dir_lower = dir_name.to_ascii_lowercase();
            let has_tech_specs = TECH_SPEC_PATTERNS
                .iter()
                .any(|pat| dir_lower.contains(pat));
            if has_tech_specs {
                let clean = normalize_dir_name(dir_name);
                issues.push(DetectedIssue {
                    issue_type: IssueType::TechSpecsInDir,
                    detail: Some(
                        serde_json::json!({
                            "old_dir": dir_name,
                            "new_dir": clean,
                        })
                        .to_string(),
                    ),
                });
            }
        }
    }

    // MISSING_YEAR_IN_DIR — album context only
    if !skip.contains(&IssueType::MissingYearInDir) && *context == AuditContext::AlbumTrack {
        // Already classified as album track, but the original (un-normalized) dir
        // might be missing the year suffix — we check the actual dir name with
        // tech specs stripped but year required.
        if let Some((_, dir_name)) = effective_album_dir_name(path) {
            if !has_year_suffix(dir_name) && !has_year_suffix(&normalize_dir_name(dir_name)) {
                issues.push(DetectedIssue {
                    issue_type: IssueType::MissingYearInDir,
                    detail: Some(
                        serde_json::json!({ "dir": dir_name }).to_string(),
                    ),
                });
            }
        }
    }

    // Parse filename and check drift / bad filename
    let parsed = parse_filename(path, context);

    // BAD_FILENAME — filename doesn't match canonical or acceptable alternates
    if !skip.contains(&IssueType::BadFilename) {
        let is_canonical = match context {
            AuditContext::AlbumTrack => parsed.track_num.is_some() && parsed.artist.is_some() && parsed.title.is_some(),
            AuditContext::LooseTrack => parsed.artist.is_some() && parsed.title.is_some(),
        };
        let is_acceptable_alternate = match context {
            AuditContext::AlbumTrack => {
                // NN. Title or NN - Title (single-artist album without artist in filename)
                parsed.track_num.is_some() && parsed.title.is_some()
            }
            AuditContext::LooseTrack => false,
        };
        if !is_canonical && !is_acceptable_alternate {
            issues.push(DetectedIssue {
                issue_type: IssueType::BadFilename,
                detail: Some(
                    serde_json::json!({
                        "filename": filename,
                        "parsed": {
                            "track_num": parsed.track_num,
                            "artist": parsed.artist,
                            "title": parsed.title,
                        },
                    })
                    .to_string(),
                ),
            });
        }
    }

    // FILENAME_TAG_DRIFT
    if !skip.contains(&IssueType::FilenameTagDrift)
        && !matches!(read_result, FileReadResult::Error { .. })
    {
        let tag_artist = get_tag_value(read_result, "artist");
        let tag_title = get_tag_value(read_result, "title");

        let mut drifts = Vec::new();

        if let (Some(fn_artist), Some(t_artist)) = (&parsed.artist, &tag_artist) {
            let fn_a = casefold_text(fn_artist.trim());
            let t_a = casefold_text(t_artist.trim());
            if !fn_a.is_empty() && !t_a.is_empty() && fn_a != t_a {
                drifts.push(serde_json::json!({
                    "field": "artist",
                    "filename": fn_artist,
                    "tag": t_artist,
                }));
            }
        }

        if let (Some(fn_title), Some(t_title)) = (&parsed.title, &tag_title) {
            // Strip (Original Mix) from filename title for comparison
            let fn_t_clean = fn_title.replace(" (Original Mix)", "");
            let fn_t = casefold_text(fn_t_clean.trim());
            let t_t = casefold_text(t_title.trim());
            if !fn_t.is_empty() && !t_t.is_empty() && fn_t != t_t {
                drifts.push(serde_json::json!({
                    "field": "title",
                    "filename": fn_title,
                    "tag": t_title,
                }));
            }
        }

        if !drifts.is_empty() {
            issues.push(DetectedIssue {
                issue_type: IssueType::FilenameTagDrift,
                detail: Some(serde_json::json!({ "drifts": drifts }).to_string()),
            });
        }
    }

    issues
}

// ---------------------------------------------------------------------------
// Scan operation
// ---------------------------------------------------------------------------

use crate::audio::AUDIO_EXTENSIONS;
const BATCH_SIZE: usize = 500;

#[derive(Debug, Serialize)]
pub struct ScanSummary {
    pub files_in_scope: usize,
    pub scanned: usize,
    pub skipped_unchanged: usize,
    pub missing_from_disk: usize,
    pub skipped_issue_types: Vec<String>,
    pub new_issues: HashMap<String, usize>,
    pub auto_resolved: HashMap<String, usize>,
    pub total_open: i64,
    pub total_resolved: i64,
    pub total_accepted: i64,
    pub total_deferred: i64,
    pub warnings: Vec<String>,
}

pub fn enforce_trailing_slash(scope: &str) -> String {
    if scope.ends_with('/') {
        scope.to_string()
    } else {
        format!("{scope}/")
    }
}

struct WalkResult {
    files: Vec<std::path::PathBuf>,
    warnings: Vec<String>,
    had_errors: bool,
}

fn walk_audio_files(scope: &Path) -> Result<WalkResult, String> {
    if !scope.is_dir() {
        return Err(format!("Not a directory: {}", scope.display()));
    }

    let mut files = Vec::new();
    let mut warnings = Vec::new();
    let mut had_errors = false;
    let mut dirs = vec![scope.to_path_buf()];

    while let Some(dir) = dirs.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) => {
                warnings.push(format!("Cannot read {}: {e}", dir.display()));
                had_errors = true;
                continue;
            }
        };

        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    warnings.push(format!("Dir entry error in {}: {e}", dir.display()));
                    had_errors = true;
                    continue;
                }
            };

            let file_type = match entry.file_type() {
                Ok(ft) => ft,
                Err(e) => {
                    warnings.push(format!("Cannot read entry type in {}: {e}", dir.display()));
                    had_errors = true;
                    continue;
                }
            };

            // Skip symlinks
            if file_type.is_symlink() {
                continue;
            }

            let path = entry.path();

            if file_type.is_dir() {
                dirs.push(path);
                continue;
            }

            if !file_type.is_file() {
                continue;
            }

            let is_audio = path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| AUDIO_EXTENSIONS.contains(&e.to_lowercase().as_str()));
            if is_audio {
                files.push(path);
            }
        }
    }

    files.sort();
    Ok(WalkResult {
        files,
        warnings,
        had_errors,
    })
}

fn file_mtime_iso(metadata: &std::fs::Metadata) -> String {
    metadata
        .modified()
        .ok()
        .and_then(|t| {
            let duration = t.duration_since(std::time::UNIX_EPOCH).ok()?;
            let dt = chrono::DateTime::from_timestamp(duration.as_secs() as i64, 0)?;
            Some(dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        })
        .unwrap_or_default()
}

fn now_iso() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

fn delete_missing_files_if_walk_complete(
    conn: &Connection,
    scope: &str,
    disk_path_set: &HashSet<String>,
    walk_had_errors: bool,
    warnings: &mut Vec<String>,
) -> Result<usize, String> {
    if walk_had_errors {
        warnings.push(
            "Skipped missing-file cleanup because filesystem walk had read errors; existing audit rows were preserved."
                .to_string(),
        );
        Ok(0)
    } else {
        store::delete_missing_audit_files(conn, scope, disk_path_set)
            .map_err(|e| format!("DB error deleting missing files: {e}"))
    }
}

pub fn scan(
    conn: &Connection,
    scope: &str,
    revalidate: bool,
    skip_issue_types: &HashSet<IssueType>,
) -> Result<ScanSummary, String> {
    let scope = enforce_trailing_slash(scope);
    if scope == "/" {
        return Err("Scope must not be empty or root (/)".to_string());
    }
    let scope_path = Path::new(&scope);

    // 1. Walk filesystem
    let walk_result = walk_audio_files(scope_path)?;
    let WalkResult {
        files: disk_files,
        mut warnings,
        had_errors: walk_had_errors,
    } = walk_result;
    let files_in_scope = disk_files.len();

    // 2. Load existing audit_files for this scope
    let existing = store::get_audit_files_in_scope(conn, &scope)
        .map_err(|e| format!("DB error loading audit files: {e}"))?;
    let existing_map: HashMap<String, store::AuditFile> = existing
        .into_iter()
        .map(|f| (f.path.clone(), f))
        .collect();

    // Track disk paths for missing-file detection
    let disk_path_set: HashSet<String> = disk_files
        .iter()
        .map(|p| p.display().to_string())
        .collect();

    // 3. Delete missing files
    let missing_from_disk = delete_missing_files_if_walk_complete(
        conn,
        &scope,
        &disk_path_set,
        walk_had_errors,
        &mut warnings,
    )?;

    let mut scanned = 0usize;
    let mut skipped_unchanged = 0usize;
    let mut new_issues: HashMap<String, usize> = HashMap::new();
    let mut auto_resolved: HashMap<String, usize> = HashMap::new();

    // 4. Process files in batches
    let mut batch_count = 0usize;
    let now = now_iso();

    // Start a transaction for the first batch
    conn.execute_batch("BEGIN TRANSACTION;")
        .map_err(|e| format!("DB error: {e}"))?;

    for file_path in &disk_files {
        let path_str = file_path.display().to_string();
        let metadata = match std::fs::metadata(file_path) {
            Ok(m) => m,
            Err(e) => {
                warnings.push(format!("Cannot stat {path_str}: {e}"));
                continue;
            }
        };
        let mtime = file_mtime_iso(&metadata);
        let size = metadata.len() as i64;

        let existing_file = existing_map.get(&path_str);

        let needs_scan = match existing_file {
            None => true, // New file
            Some(ef) => {
                if revalidate {
                    true
                } else {
                    ef.file_mtime != mtime || ef.file_size != size
                }
            }
        };

        if !needs_scan {
            skipped_unchanged += 1;
        } else {
            // Read tags
            let read_result = tags::read_file_tags(file_path, None, false);

            // Determine context
            let context = classify_track_context(file_path);

            // Run checks
            let mut detected: Vec<DetectedIssue> = Vec::new();
            if !matches!(read_result, FileReadResult::Error { .. }) {
                detected.extend(check_tags(file_path, &read_result, &context, skip_issue_types));
                detected.extend(check_filename(
                    file_path,
                    &read_result,
                    &context,
                    skip_issue_types,
                ));
            }

            // Upsert audit_file
            store::upsert_audit_file(conn, &path_str, &now, &mtime, size)
                .map_err(|e| format!("DB error upserting file: {e}"))?;

            // Upsert detected issues
            let detected_types: Vec<&str> = detected.iter().map(|d| d.issue_type.as_str()).collect();
            for issue in &detected {
                store::upsert_audit_issue(
                    conn,
                    &path_str,
                    issue.issue_type.as_str(),
                    issue.detail.as_deref(),
                    "open",
                    &now,
                )
                .map_err(|e| format!("DB error upserting issue: {e}"))?;

                *new_issues.entry(issue.issue_type.to_string()).or_insert(0) += 1;
            }

            // Auto-resolve issues no longer detected (for changed/re-read files).
            // Skip when file read errored — we don't know the true state.
            if existing_file.is_some() && !matches!(read_result, FileReadResult::Error { .. }) {
                // Skipped issue types should not be auto-resolved — we didn't check them
                let mut types_still_open: Vec<&str> = detected_types.clone();
                for skip_type in skip_issue_types {
                    let s = skip_type.as_str();
                    if !types_still_open.contains(&s) {
                        types_still_open.push(s);
                    }
                }

                let resolved_count = store::mark_issues_resolved_for_path(
                    conn,
                    &path_str,
                    &types_still_open,
                    &now,
                )
                .map_err(|e| format!("DB error resolving issues: {e}"))?;
                if resolved_count > 0 {
                    *auto_resolved.entry("_total".to_string()).or_insert(0) += resolved_count;
                }
            }

            scanned += 1;
        }

        batch_count += 1;
        if batch_count >= BATCH_SIZE {
            conn.execute_batch("COMMIT; BEGIN TRANSACTION;")
                .map_err(|e| format!("DB error committing batch: {e}"))?;
            batch_count = 0;
        }
    }

    // Commit final batch
    conn.execute_batch("COMMIT;")
        .map_err(|e| format!("DB error committing final batch: {e}"))?;

    // 5. Build summary from DB
    let summary = store::get_audit_summary(conn, &scope)
        .map_err(|e| format!("DB error getting summary: {e}"))?;

    let mut total_open = 0i64;
    let mut total_resolved = 0i64;
    let mut total_accepted = 0i64;
    let mut total_deferred = 0i64;

    for (_, status, count) in &summary.by_type_status {
        match AuditStatus::from_str(status) {
            Some(AuditStatus::Open) => total_open += count,
            Some(AuditStatus::Resolved) => total_resolved += count,
            Some(AuditStatus::Accepted) => total_accepted += count,
            Some(AuditStatus::Deferred) => total_deferred += count,
            None => {}
        }
    }

    let skipped_names: Vec<String> = skip_issue_types.iter().map(|t| t.to_string()).collect();

    Ok(ScanSummary {
        files_in_scope,
        scanned,
        skipped_unchanged,
        missing_from_disk,
        skipped_issue_types: skipped_names,
        new_issues,
        auto_resolved,
        total_open,
        total_resolved,
        total_accepted,
        total_deferred,
        warnings,
    })
}

// ---------------------------------------------------------------------------
// Query, resolve, summary — thin wrappers
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct IssueRecord {
    pub id: i64,
    pub path: String,
    pub issue_type: String,
    pub detail: Option<serde_json::Value>,
    pub status: String,
    pub resolution: Option<String>,
    pub note: Option<String>,
    pub created_at: String,
    pub resolved_at: Option<String>,
}

fn store_issue_to_record(i: store::AuditIssue) -> IssueRecord {
    let detail = i
        .detail
        .as_deref()
        .and_then(|d| serde_json::from_str(d).ok());
    IssueRecord {
        id: i.id,
        path: i.path,
        issue_type: i.issue_type,
        detail,
        status: i.status,
        resolution: i.resolution,
        note: i.note,
        created_at: i.created_at,
        resolved_at: i.resolved_at,
    }
}

pub fn query_issues(
    conn: &Connection,
    scope: &str,
    status: Option<&str>,
    issue_type: Option<&str>,
    limit: u32,
    offset: u32,
) -> Result<Vec<IssueRecord>, String> {
    let scope = enforce_trailing_slash(scope);
    if scope == "/" {
        return Err("Scope must not be empty or root (/)".to_string());
    }
    let issues = store::get_audit_issues(conn, &scope, status, issue_type, limit, offset)
        .map_err(|e| format!("DB error: {e}"))?;
    Ok(issues.into_iter().map(store_issue_to_record).collect())
}

pub fn resolve_issues(
    conn: &Connection,
    ids: &[i64],
    resolution: &str,
    note: Option<&str>,
) -> Result<usize, String> {
    let res = Resolution::from_str(resolution)
        .filter(|r| !matches!(r, Resolution::Fixed))
        .ok_or_else(|| {
            format!(
                "Invalid resolution \"{resolution}\". Must be one of: \
                 accepted_as_is, wont_fix, deferred"
            )
        })?;
    let now = now_iso();
    store::resolve_audit_issues(conn, ids, res, note, &now)
        .map_err(|e| format!("DB error: {e}"))
}

#[derive(Debug, Serialize)]
pub struct SummaryReport {
    pub scope: String,
    pub by_type: HashMap<String, HashMap<String, i64>>,
    pub total_open: i64,
    pub total_resolved: i64,
    pub total_accepted: i64,
    pub total_deferred: i64,
}

pub fn get_summary(conn: &Connection, scope: &str) -> Result<SummaryReport, String> {
    let scope = enforce_trailing_slash(scope);
    if scope == "/" {
        return Err("Scope must not be empty or root (/)".to_string());
    }
    let summary = store::get_audit_summary(conn, &scope)
        .map_err(|e| format!("DB error: {e}"))?;

    let mut by_type: HashMap<String, HashMap<String, i64>> = HashMap::new();
    let mut total_open = 0i64;
    let mut total_resolved = 0i64;
    let mut total_accepted = 0i64;
    let mut total_deferred = 0i64;

    for (issue_type, status, count) in &summary.by_type_status {
        by_type
            .entry(issue_type.clone())
            .or_default()
            .insert(status.clone(), *count);
        match AuditStatus::from_str(status) {
            Some(AuditStatus::Open) => total_open += count,
            Some(AuditStatus::Resolved) => total_resolved += count,
            Some(AuditStatus::Accepted) => total_accepted += count,
            Some(AuditStatus::Deferred) => total_deferred += count,
            None => {}
        }
    }

    Ok(SummaryReport {
        scope,
        by_type,
        total_open,
        total_resolved,
        total_accepted,
        total_deferred,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- classify_track_context --

    #[test]
    fn classify_album_track_with_year() {
        let p = Path::new("/music/Artist/Album Name (2024)/01 Artist - Track.flac");
        assert_eq!(classify_track_context(p), AuditContext::AlbumTrack);
    }

    #[test]
    fn classify_album_track_with_tech_specs_and_year() {
        let p = Path::new("/music/Artist/Album [FLAC] (2024)/01 Artist - Track.flac");
        assert_eq!(classify_track_context(p), AuditContext::AlbumTrack);
    }

    #[test]
    fn classify_loose_track_in_play_dir() {
        let p = Path::new("/music/play/Artist - Track.wav");
        assert_eq!(classify_track_context(p), AuditContext::LooseTrack);
    }

    #[test]
    fn classify_loose_track_no_year() {
        let p = Path::new("/music/Artist/SomeDir/Artist - Track.flac");
        assert_eq!(classify_track_context(p), AuditContext::LooseTrack);
    }

    #[test]
    fn classify_disc_subdir() {
        let p = Path::new("/music/Artist/Album (2020)/CD1/01 Artist - Track.flac");
        assert_eq!(classify_track_context(p), AuditContext::AlbumTrack);
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

    fn make_single(fields: &[(&str, &str)]) -> FileReadResult {
        let mut tags = HashMap::new();
        for &f in tags::ALL_FIELDS {
            tags.insert(f.to_string(), None);
        }
        for &(k, v) in fields {
            tags.insert(k.to_string(), Some(v.to_string()));
        }
        FileReadResult::Single {
            path: "/test/track.flac".to_string(),
            format: "flac".to_string(),
            tag_type: "vorbis_comment".to_string(),
            tags,
            cover_art: None,
        }
    }

    fn make_wav(
        id3v2_fields: &[(&str, &str)],
        riff_fields: &[(&str, &str)],
        tag3_missing: Vec<String>,
    ) -> FileReadResult {
        let mut id3v2 = HashMap::new();
        let mut riff_info = HashMap::new();
        for &f in tags::ALL_FIELDS {
            id3v2.insert(f.to_string(), None);
            riff_info.insert(f.to_string(), None);
        }
        for &(k, v) in id3v2_fields {
            id3v2.insert(k.to_string(), Some(v.to_string()));
        }
        for &(k, v) in riff_fields {
            riff_info.insert(k.to_string(), Some(v.to_string()));
        }
        FileReadResult::Wav {
            path: "/test/track.wav".to_string(),
            format: "wav".to_string(),
            id3v2,
            riff_info,
            tag3_missing,
            cover_art: None,
        }
    }

    #[test]
    fn check_tags_empty_artist() {
        let result = make_single(&[("title", "Track")]);
        let issues = check_tags(
            Path::new("/test/track.flac"),
            &result,
            &AuditContext::LooseTrack,
            &HashSet::new(),
        );
        assert!(issues.iter().any(|i| i.issue_type == IssueType::EmptyArtist));
    }

    #[test]
    fn check_tags_empty_title() {
        let result = make_single(&[("artist", "Artist")]);
        let issues = check_tags(
            Path::new("/test/track.flac"),
            &result,
            &AuditContext::LooseTrack,
            &HashSet::new(),
        );
        assert!(issues.iter().any(|i| i.issue_type == IssueType::EmptyTitle));
    }

    #[test]
    fn check_tags_album_missing_fields() {
        let result = make_single(&[("artist", "A"), ("title", "T")]);
        let issues = check_tags(
            Path::new("/test/track.flac"),
            &result,
            &AuditContext::AlbumTrack,
            &HashSet::new(),
        );
        assert!(issues
            .iter()
            .any(|i| i.issue_type == IssueType::MissingTrackNum));
        assert!(issues
            .iter()
            .any(|i| i.issue_type == IssueType::MissingAlbum));
        assert!(issues
            .iter()
            .any(|i| i.issue_type == IssueType::MissingYear));
    }

    #[test]
    fn check_tags_album_all_present() {
        let result = make_single(&[
            ("artist", "A"),
            ("title", "T"),
            ("track", "1"),
            ("album", "Al"),
            ("year", "2024"),
        ]);
        let issues = check_tags(
            Path::new("/test/track.flac"),
            &result,
            &AuditContext::AlbumTrack,
            &HashSet::new(),
        );
        assert!(issues.is_empty());
    }

    #[test]
    fn check_tags_artist_in_title() {
        let result = make_single(&[("artist", "Burial"), ("title", "Burial - Archangel")]);
        let issues = check_tags(
            Path::new("/test/track.flac"),
            &result,
            &AuditContext::LooseTrack,
            &HashSet::new(),
        );
        let ait = issues
            .iter()
            .find(|i| i.issue_type == IssueType::ArtistInTitle)
            .expect("should detect artist in title");
        let detail: serde_json::Value = serde_json::from_str(ait.detail.as_ref().unwrap()).unwrap();
        assert_eq!(detail["new_title"], "Archangel");
    }

    #[test]
    fn check_tags_artist_in_title_case_insensitive() {
        let result = make_single(&[("artist", "burial"), ("title", "Burial - Archangel")]);
        let issues = check_tags(
            Path::new("/test/track.flac"),
            &result,
            &AuditContext::LooseTrack,
            &HashSet::new(),
        );
        assert!(issues
            .iter()
            .any(|i| i.issue_type == IssueType::ArtistInTitle));
    }

    #[test]
    fn check_tags_wav_tag3_missing() {
        let result = make_wav(
            &[("artist", "A"), ("title", "T")],
            &[("title", "T")],
            vec!["artist".to_string()],
        );
        let issues = check_tags(
            Path::new("/test/track.wav"),
            &result,
            &AuditContext::LooseTrack,
            &HashSet::new(),
        );
        assert!(issues
            .iter()
            .any(|i| i.issue_type == IssueType::WavTag3Missing));
    }

    #[test]
    fn check_tags_wav_tag_drift() {
        let result = make_wav(
            &[("artist", "Correct")],
            &[("artist", "Wrong")],
            vec![],
        );
        let issues = check_tags(
            Path::new("/test/track.wav"),
            &result,
            &AuditContext::LooseTrack,
            &HashSet::new(),
        );
        assert!(issues
            .iter()
            .any(|i| i.issue_type == IssueType::WavTagDrift));
    }

    #[test]
    fn check_tags_genre_set() {
        let result = make_single(&[("artist", "A"), ("title", "T"), ("genre", "House")]);
        let issues = check_tags(
            Path::new("/test/track.flac"),
            &result,
            &AuditContext::LooseTrack,
            &HashSet::new(),
        );
        assert!(issues
            .iter()
            .any(|i| i.issue_type == IssueType::GenreSet));
    }

    #[test]
    fn check_tags_no_tags() {
        let result = make_single(&[]);
        let issues = check_tags(
            Path::new("/test/track.flac"),
            &result,
            &AuditContext::LooseTrack,
            &HashSet::new(),
        );
        assert!(issues.iter().any(|i| i.issue_type == IssueType::NoTags));
        // Should NOT also report EMPTY_ARTIST etc when NO_TAGS fires
        assert!(!issues
            .iter()
            .any(|i| i.issue_type == IssueType::EmptyArtist));
    }

    #[test]
    fn check_tags_skip_genre() {
        let result = make_single(&[("artist", "A"), ("title", "T"), ("genre", "House")]);
        let skip: HashSet<IssueType> = [IssueType::GenreSet].into();
        let issues = check_tags(
            Path::new("/test/track.flac"),
            &result,
            &AuditContext::LooseTrack,
            &skip,
        );
        assert!(!issues.iter().any(|i| i.issue_type == IssueType::GenreSet));
    }

    // -- check_filename --

    #[test]
    fn check_filename_original_mix() {
        let result = make_single(&[("artist", "A"), ("title", "T")]);
        let issues = check_filename(
            Path::new("/test/A - Track (Original Mix).flac"),
            &result,
            &AuditContext::LooseTrack,
            &HashSet::new(),
        );
        assert!(issues
            .iter()
            .any(|i| i.issue_type == IssueType::OriginalMixSuffix));
    }

    #[test]
    fn check_filename_tech_specs() {
        let result = make_single(&[("artist", "A"), ("title", "T")]);
        let issues = check_filename(
            Path::new("/test/Album [FLAC]/01 A - T.flac"),
            &result,
            &AuditContext::LooseTrack,
            &HashSet::new(),
        );
        assert!(issues
            .iter()
            .any(|i| i.issue_type == IssueType::TechSpecsInDir));
    }

    #[test]
    fn check_filename_tag_drift() {
        let result = make_single(&[("artist", "RealArtist"), ("title", "RealTitle")]);
        let issues = check_filename(
            Path::new("/music/play/WrongArtist - WrongTitle.flac"),
            &result,
            &AuditContext::LooseTrack,
            &HashSet::new(),
        );
        assert!(issues
            .iter()
            .any(|i| i.issue_type == IssueType::FilenameTagDrift));
    }

    #[test]
    fn check_filename_no_drift_when_matching() {
        let result = make_single(&[("artist", "Burial"), ("title", "Archangel")]);
        let issues = check_filename(
            Path::new("/music/play/Burial - Archangel.flac"),
            &result,
            &AuditContext::LooseTrack,
            &HashSet::new(),
        );
        assert!(!issues
            .iter()
            .any(|i| i.issue_type == IssueType::FilenameTagDrift));
    }

    #[test]
    fn check_filename_no_drift_with_unicode_casefold_artist() {
        let result = make_single(&[("artist", "SS"), ("title", "Track")]);
        let issues = check_filename(
            Path::new("/music/play/ß - Track.flac"),
            &result,
            &AuditContext::LooseTrack,
            &HashSet::new(),
        );
        assert!(!issues
            .iter()
            .any(|i| i.issue_type == IssueType::FilenameTagDrift));
    }

    #[test]
    fn check_filename_no_drift_with_unicode_casefold_title() {
        let result = make_single(&[("artist", "Artist"), ("title", "STRASSE")]);
        let issues = check_filename(
            Path::new("/music/play/Artist - Straße.flac"),
            &result,
            &AuditContext::LooseTrack,
            &HashSet::new(),
        );
        assert!(!issues
            .iter()
            .any(|i| i.issue_type == IssueType::FilenameTagDrift));
    }

    // -- normalize_dir_name --

    #[test]
    fn normalize_strips_tech_specs() {
        assert_eq!(normalize_dir_name("Album [FLAC] (2024)"), "Album (2024)");
        assert_eq!(normalize_dir_name("Album [WAV] 24-96"), "Album");
    }

    // -- IssueType round-trip --

    #[test]
    fn issue_type_str_round_trip() {
        use strum::IntoEnumIterator;
        for it in IssueType::iter() {
            let s = it.as_str();
            let back: IssueType = s
                .parse()
                .unwrap_or_else(|_| panic!("No IssueType for \"{s}\""));
            assert_eq!(it, back);
        }
    }

    // -- safety_tier --

    #[test]
    fn safety_tiers() {
        use strum::IntoEnumIterator;
        // Every variant has a tier — new variants cause a compile error in safety_tier()
        for it in IssueType::iter() {
            let _ = it.safety_tier();
        }
        // Spot-check specific tiers
        assert_eq!(IssueType::ArtistInTitle.safety_tier(), SafetyTier::Safe);
        assert_eq!(IssueType::WavTag3Missing.safety_tier(), SafetyTier::Safe);
        assert_eq!(IssueType::WavTagDrift.safety_tier(), SafetyTier::Safe);
        assert_eq!(
            IssueType::OriginalMixSuffix.safety_tier(),
            SafetyTier::RenameSafe
        );
        assert_eq!(
            IssueType::TechSpecsInDir.safety_tier(),
            SafetyTier::RenameSafe
        );
        assert_eq!(IssueType::EmptyArtist.safety_tier(), SafetyTier::Review);
        assert_eq!(IssueType::GenreSet.safety_tier(), SafetyTier::Review);
    }

    // -- Bug-fix regression tests --

    /// Helper: build a tag map from key-value pairs, filling missing fields with None.
    fn make_tags(fields: &[(&str, &str)]) -> HashMap<String, Option<String>> {
        let mut tags = HashMap::new();
        for &f in tags::ALL_FIELDS {
            tags.insert(f.to_string(), None);
        }
        for &(k, v) in fields {
            tags.insert(k.to_string(), Some(v.to_string()));
        }
        tags
    }

    // Finding 1: MISSING_YEAR requires both year and date empty
    #[test]
    fn check_tags_missing_year_requires_both_empty() {
        // If year is empty but date is set, should NOT flag MISSING_YEAR
        let tags = make_tags(&[("artist", "A"), ("title", "T"), ("album", "Alb"), ("track", "1"), ("date", "2024")]);
        let result = FileReadResult::Single {
            path: "/music/Artist/Album (2024)/01 A - T.flac".to_string(),
            format: "FLAC".to_string(),
            tag_type: "VorbisComments".to_string(),
            tags,
            cover_art: None,
        };
        let issues = check_tags(Path::new("/x"), &result, &AuditContext::AlbumTrack, &HashSet::new());
        assert!(!issues.iter().any(|i| i.issue_type == IssueType::MissingYear));
    }

    // Finding 2: Multi-byte UTF-8 in filename doesn't panic
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
    fn check_tags_artist_in_title_new_title_correct() {
        // Ensure the new_title slice is correct even with varying case
        let tags = make_tags(&[("artist", "DJ Test"), ("title", "DJ Test - The Track")]);
        let result = FileReadResult::Single {
            path: "/x.flac".to_string(),
            format: "FLAC".to_string(),
            tag_type: "VorbisComments".to_string(),
            tags,
            cover_art: None,
        };
        let issues = check_tags(Path::new("/x"), &result, &AuditContext::LooseTrack, &HashSet::new());
        let ait = issues.iter().find(|i| i.issue_type == IssueType::ArtistInTitle).expect("should detect");
        let detail: serde_json::Value = serde_json::from_str(ait.detail.as_ref().unwrap()).unwrap();
        assert_eq!(detail["new_title"], "The Track");
    }

    #[test]
    fn check_tags_artist_in_title_uses_unicode_casefold() {
        let tags = make_tags(&[("artist", "ß"), ("title", "SS - Track")]);
        let result = FileReadResult::Single {
            path: "/x.flac".to_string(),
            format: "FLAC".to_string(),
            tag_type: "VorbisComments".to_string(),
            tags,
            cover_art: None,
        };
        let issues = check_tags(
            Path::new("/x"),
            &result,
            &AuditContext::LooseTrack,
            &HashSet::new(),
        );
        assert!(issues
            .iter()
            .any(|i| i.issue_type == IssueType::ArtistInTitle));
    }

    #[test]
    fn check_tags_artist_in_title_artist_contains_separator() {
        let tags = make_tags(&[("artist", "AC - DC"), ("title", "AC - DC - Thunderstruck")]);
        let result = FileReadResult::Single {
            path: "/x.flac".to_string(),
            format: "FLAC".to_string(),
            tag_type: "VorbisComments".to_string(),
            tags,
            cover_art: None,
        };
        let issues = check_tags(
            Path::new("/x"),
            &result,
            &AuditContext::LooseTrack,
            &HashSet::new(),
        );
        let ait = issues
            .iter()
            .find(|i| i.issue_type == IssueType::ArtistInTitle)
            .expect("should detect artist in title");
        let detail: serde_json::Value = serde_json::from_str(ait.detail.as_ref().unwrap()).unwrap();
        assert_eq!(detail["new_title"], "Thunderstruck");
    }

    // Finding 7: Empty scope rejected
    #[test]
    fn scan_rejects_empty_scope() {
        let result = enforce_trailing_slash("");
        assert_eq!(result, "/");
        // The scan function should reject this — we can't actually call scan here
        // without a real DB, so just verify enforce_trailing_slash behavior
    }

    // Finding 9: NN - Title parsing
    #[test]
    fn parse_album_nn_dash_title() {
        let p = Path::new("/music/Artist/Album (2024)/05 - Invisible Dance.flac");
        let parsed = parse_filename(p, &AuditContext::AlbumTrack);
        assert_eq!(parsed.track_num.as_deref(), Some("05"));
        assert_eq!(parsed.title.as_deref(), Some("Invisible Dance"));
        assert_eq!(parsed.artist, None); // Alternate format, no artist in filename
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
    fn check_filename_disc_subdir_uses_album_dir() {
        // File in CD1 subdir under album dir with year — should NOT flag MISSING_YEAR_IN_DIR
        let p = Path::new("/music/Artist/Album (2020)/CD1/01 Artist - Track.flac");
        let tags = make_tags(&[("artist", "Artist"), ("title", "Track")]);
        let result = FileReadResult::Single {
            path: p.to_str().unwrap().to_string(),
            format: "FLAC".to_string(),
            tag_type: "VorbisComments".to_string(),
            tags,
            cover_art: None,
        };
        let issues = check_filename(p, &result, &AuditContext::AlbumTrack, &HashSet::new());
        assert!(!issues.iter().any(|i| i.issue_type == IssueType::MissingYearInDir),
            "Should not flag MISSING_YEAR_IN_DIR when album dir has year suffix");
    }

    #[test]
    fn skip_missing_cleanup_when_walk_has_errors() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("internal.sqlite3");
        let conn = store::open(db_path.to_str().unwrap()).unwrap();

        store::upsert_audit_file(&conn, "/music/a.flac", "t1", "m1", 100).unwrap();

        let disk_path_set: HashSet<String> = HashSet::new();
        let mut warnings = Vec::new();
        let removed = delete_missing_files_if_walk_complete(
            &conn,
            "/music/",
            &disk_path_set,
            true,
            &mut warnings,
        )
        .unwrap();

        assert_eq!(removed, 0);
        let files = store::get_audit_files_in_scope(&conn, "/music/").unwrap();
        assert_eq!(files.len(), 1);
        assert!(warnings
            .iter()
            .any(|w| w.contains("Skipped missing-file cleanup")));
    }

    #[cfg(unix)]
    #[test]
    fn scan_skips_missing_cleanup_when_walk_hits_unreadable_subdir() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("internal.sqlite3");
        let conn = store::open(db_path.to_str().unwrap()).unwrap();

        let ok_file = dir.path().join("ok.flac");
        std::fs::write(&ok_file, b"not-audio").unwrap();

        let blocked_dir = dir.path().join("blocked");
        std::fs::create_dir(&blocked_dir).unwrap();
        let blocked_file = blocked_dir.join("hidden.flac");
        std::fs::write(&blocked_file, b"not-audio").unwrap();

        let ok_path = ok_file.to_str().unwrap();
        let blocked_path = blocked_file.to_str().unwrap();
        store::upsert_audit_file(&conn, ok_path, "t1", "m1", 1).unwrap();
        store::upsert_audit_file(&conn, blocked_path, "t1", "m1", 1).unwrap();

        let original_perms = std::fs::metadata(&blocked_dir).unwrap().permissions();
        let mut no_access = original_perms.clone();
        no_access.set_mode(0o000);
        std::fs::set_permissions(&blocked_dir, no_access).unwrap();

        let scan_result = scan(&conn, dir.path().to_str().unwrap(), false, &HashSet::new());

        std::fs::set_permissions(&blocked_dir, original_perms).unwrap();

        let summary = scan_result.expect("scan should continue with warnings");
        assert_eq!(summary.missing_from_disk, 0);
        assert!(summary
            .warnings
            .iter()
            .any(|w| w.contains("Cannot read")));
        assert!(summary
            .warnings
            .iter()
            .any(|w| w.contains("Skipped missing-file cleanup")));
        assert!(store::get_audit_file(&conn, blocked_path).unwrap().is_some());
    }
}
