//! Pure audit checks over a transport-neutral tag projection.

use std::collections::HashSet;
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;
use unicode_casefold::UnicodeCaseFold;

use super::filename::{
    TECH_SPEC_RE, ancestor_has_year, effective_album_dir_name, has_bare_year, has_year_range,
    has_year_suffix, normalize_dir_name, parse_filename,
};
use super::{AuditContext, IssueType, TagSnapshot};

const AUDIT_FIELDS: &[&str] = &[
    "artist",
    "title",
    "album",
    "album_artist",
    "genre",
    "year",
    "track",
    "disc",
    "comment",
    "publisher",
    "bpm",
    "key",
    "composer",
    "remixer",
];

#[derive(Debug, Clone)]
pub struct DetectedIssue {
    pub issue_type: IssueType,
    pub detail: Option<String>,
}

// ---------------------------------------------------------------------------
// Convention checks — pure functions
// ---------------------------------------------------------------------------

/// Get a field value from the primary tag layer in a TagSnapshot (WAV uses ID3v2).
pub(crate) fn get_tag_value(result: &TagSnapshot, field: &str) -> Option<String> {
    match result {
        TagSnapshot::Single { tags, .. } => tags.get(field).and_then(std::clone::Clone::clone),
        TagSnapshot::Wav { id3v2, .. } => id3v2.get(field).and_then(std::clone::Clone::clone),
        TagSnapshot::Error => None,
    }
}

pub(crate) fn tag_is_empty(result: &TagSnapshot, field: &str) -> bool {
    get_tag_value(result, field)
        .as_ref()
        .is_none_or(|v| v.trim().is_empty())
}

pub(crate) fn all_tags_empty(result: &TagSnapshot) -> bool {
    AUDIT_FIELDS.iter().all(|&f| tag_is_empty(result, f))
}

pub(crate) fn is_wav(result: &TagSnapshot) -> bool {
    matches!(result, TagSnapshot::Wav { .. })
}

pub(crate) fn casefold_text(s: &str) -> String {
    s.case_fold().collect()
}

/// Normalize only `/` (the sole macOS-forbidden filename character) before
/// drift comparison.  All other special characters (`: ? " * | < >`) are
/// valid in macOS filenames, so drift involving them is real and actionable
/// — the file can and should be renamed to match the tag.
pub(crate) fn normalize_for_drift(s: &str) -> String {
    s.replace('/', "-")
}

pub fn check_tags(
    path: &Path,
    read_result: &TagSnapshot,
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

    if !skip.contains(&IssueType::EmptyArtist) && tag_is_empty(read_result, "artist") {
        issues.push(DetectedIssue {
            issue_type: IssueType::EmptyArtist,
            detail: None,
        });
    }

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

        // "date" is not in AUDIT_FIELDS and is never populated for non-WAV files,
        // so checking it was a no-op. Fire MissingYear based on "year" alone.
        if !skip.contains(&IssueType::MissingYear) && tag_is_empty(read_result, "year") {
            issues.push(DetectedIssue {
                issue_type: IssueType::MissingYear,
                detail: None,
            });
        }
    }

    if !skip.contains(&IssueType::ArtistInTitle)
        && let (Some(artist), Some(title)) = (
            get_tag_value(read_result, "artist"),
            get_tag_value(read_result, "title"),
        )
    {
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

    // WAV-specific checks
    if is_wav(read_result)
        && let TagSnapshot::Wav {
            tag3_missing,
            id3v2,
            riff_info,
            ..
        } = read_result
    {
        if !skip.contains(&IssueType::WavTag3Missing) && !tag3_missing.is_empty() {
            issues.push(DetectedIssue {
                issue_type: IssueType::WavTag3Missing,
                detail: Some(serde_json::json!({ "fields": tag3_missing }).to_string()),
            });
        }

        if !skip.contains(&IssueType::WavTagDrift) {
            let mut drifted = Vec::new();
            for field in &["artist", "title", "album", "genre", "year", "comment"] {
                let id3v2_value = id3v2.get(*field).and_then(|v| v.as_deref()).map(str::trim);
                let riff_info_value = riff_info
                    .get(*field)
                    .and_then(|v| v.as_deref())
                    .map(str::trim);
                if let (Some(v2_val), Some(ri_val)) = (id3v2_value, riff_info_value)
                    && v2_val != ri_val
                {
                    drifted.push(serde_json::json!({
                        "field": field,
                        "id3v2": v2_val,
                        "riff_info": ri_val,
                    }));
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
    read_result: &TagSnapshot,
    context: &AuditContext,
    skip: &HashSet<IssueType>,
) -> Vec<DetectedIssue> {
    let mut issues = Vec::new();

    let filename = match path.file_name().and_then(|n| n.to_str()) {
        Some(n) => n,
        None => return issues,
    };

    // ORIGINAL_MIX_SUFFIX (case-insensitive) — matches (Original), (Original Mix), (Original Version)
    static ORIGINAL_MIX_RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)\s*\(Original(?:\s+(?:Mix|Version))?\)")
            .expect("ORIGINAL_MIX_RE must compile")
    });
    if !skip.contains(&IssueType::OriginalMixSuffix) && ORIGINAL_MIX_RE.is_match(filename) {
        let new_name = ORIGINAL_MIX_RE.replace_all(filename, "");
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

    if !skip.contains(&IssueType::TechSpecsInDir)
        && let Some((_, dir_name)) = effective_album_dir_name(path)
    {
        let has_tech_specs = TECH_SPEC_RE.is_match(dir_name);
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

    // MISSING_YEAR_IN_DIR — album context only
    if !skip.contains(&IssueType::MissingYearInDir)
        && *context == AuditContext::AlbumTrack
        && let Some((_, dir_name)) = effective_album_dir_name(path)
        && !has_year_suffix(dir_name)
        && !has_year_suffix(&normalize_dir_name(dir_name))
        && !has_year_range(dir_name)
        && !has_bare_year(dir_name)
        && !ancestor_has_year(path)
    {
        issues.push(DetectedIssue {
            issue_type: IssueType::MissingYearInDir,
            detail: Some(serde_json::json!({ "dir": dir_name }).to_string()),
        });
    }

    let parsed = parse_filename(path, context);

    // BAD_FILENAME — filename doesn't match canonical or acceptable alternates
    if !skip.contains(&IssueType::BadFilename) {
        let is_canonical = match context {
            AuditContext::AlbumTrack => {
                parsed.track_num.is_some() && parsed.artist.is_some() && parsed.title.is_some()
            }
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

    if !skip.contains(&IssueType::FilenameTagDrift) && !matches!(read_result, TagSnapshot::Error) {
        let tag_artist = get_tag_value(read_result, "artist");
        let tag_title = get_tag_value(read_result, "title");

        let mut drifts = Vec::new();

        if let (Some(fn_artist), Some(t_artist)) = (&parsed.artist, &tag_artist) {
            let filename_artist_folded = casefold_text(&normalize_for_drift(fn_artist.trim()));
            let tag_artist_folded = casefold_text(&normalize_for_drift(t_artist.trim()));
            if !filename_artist_folded.is_empty()
                && !tag_artist_folded.is_empty()
                && filename_artist_folded != tag_artist_folded
            {
                drifts.push(serde_json::json!({
                    "field": "artist",
                    "filename": fn_artist,
                    "tag": t_artist,
                }));
            }
        }

        if let (Some(fn_title), Some(t_title)) = (&parsed.title, &tag_title) {
            // Strip (Original Mix) from both sides for comparison (case-insensitive)
            let fn_t_clean = ORIGINAL_MIX_RE.replace_all(fn_title, "").into_owned();
            let tag_t_clean = ORIGINAL_MIX_RE.replace_all(t_title, "").into_owned();
            let filename_title_folded = casefold_text(&normalize_for_drift(fn_t_clean.trim()));
            let tag_title_folded = casefold_text(&normalize_for_drift(tag_t_clean.trim()));
            if !filename_title_folded.is_empty()
                && !tag_title_folded.is_empty()
                && filename_title_folded != tag_title_folded
            {
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
