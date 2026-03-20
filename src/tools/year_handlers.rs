use std::path::Path;

use rmcp::ErrorData as McpError;
use rmcp::model::{CallToolResult, Content};
use schemars::JsonSchema;
use serde::Deserialize;

use super::*;
use crate::db;
use crate::normalize;
use crate::store;
use crate::tags;
use crate::types::TrackChange;

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct BackfillYearsParams {
    #[schemars(description = "Preview changes without staging (default false)")]
    pub dry_run: Option<bool>,
}

fn parse_year_str(s: &str) -> Option<i32> {
    // Accept "2019", "2019-01-15", etc. — take first 4 digits.
    let trimmed = s.trim();
    if trimmed.len() < 4 {
        return None;
    }
    let digits: String = trimmed.chars().take(4).collect();
    let year: i32 = digits.parse().ok()?;
    (1900..=2099).contains(&year).then_some(year)
}

/// Try to read a year value from the audio file's metadata tags.
fn year_from_file_tags(path: &str) -> Option<i32> {
    let fields = ["year".to_string()];
    let result = tags::read_file_tags(Path::new(path), Some(&fields), false);
    let year_str = match result {
        tags::FileReadResult::Single { ref tags, .. } => tags.get("year")?.clone(),
        tags::FileReadResult::Wav {
            ref id3v2,
            ref riff_info,
            ..
        } => id3v2
            .get("year")
            .or_else(|| riff_info.get("year"))?
            .clone(),
        tags::FileReadResult::Error { .. } => return None,
    };
    year_str.and_then(|s| parse_year_str(&s))
}

/// Extract a year from a `(YYYY)` suffix in the parent directory name.
fn year_from_folder_path(file_path: &str) -> Option<i32> {
    let path = Path::new(file_path);
    let parent = path.parent()?;
    let dir_name = parent.file_name()?.to_str()?;
    let trimmed = dir_name.trim_end();
    if trimmed.len() < 6 {
        return None;
    }
    if trimmed.as_bytes()[trimmed.len() - 1] != b')' {
        return None;
    }
    let open = trimmed.rfind('(')?;
    let inside = &trimmed[open + 1..trimmed.len() - 1];
    if let Some(prefix) = inside.get(..4) {
        if prefix.bytes().all(|b| b.is_ascii_digit()) {
            let year: i32 = prefix.parse().ok()?;
            return (1900..=2099).contains(&year).then_some(year);
        }
    }
    None
}

pub(super) fn handle_backfill_years(
    server: &ReklawdboxServer,
    params: BackfillYearsParams,
) -> Result<CallToolResult, McpError> {
    let dry_run = params.dry_run.unwrap_or(false);

    let rb_conn = server.rekordbox_conn()?;
    let search_params = db::SearchParams {
        query: None,
        artist: None,
        genre: None,
        rating_min: None,
        bpm_min: None,
        bpm_max: None,
        key: None,
        playlist: None,
        has_genre: None,
        has_label: None,
        label: None,
        path: None,
        path_prefix: None,
        added_after: None,
        added_before: None,
        exclude_samples: true,
        limit: None,
        offset: None,
    };
    let tracks = db::search_tracks_unbounded(&rb_conn, &search_params)
        .map_err(|e| mcp_internal_error(format!("DB error: {e}")))?;
    drop(rb_conn);

    let store_conn = server.cache_store_conn()?;

    let mut filled_file_tags = 0usize;
    let mut filled_folder_path = 0usize;
    let mut filled_discogs = 0usize;
    let mut filled_beatport = 0usize;
    let mut filled_musicbrainz = 0usize;
    let mut filled_bandcamp = 0usize;
    let mut already_set = 0usize;
    let mut conflicts = Vec::new();
    let mut remaining_year_zero = Vec::new();
    let mut to_stage = Vec::new();

    for track in &tracks {
        if track.year == 0 {
            // Priority cascade: file tags → folder path → Discogs → Beatport → MusicBrainz.
            if let Some(year) = year_from_file_tags(&track.file_path) {
                filled_file_tags += 1;
                to_stage.push(TrackChange {
                    track_id: track.id.clone(),
                    genre: None,
                    comments: None,
                    rating: None,
                    color: None,
                    label: None,
                    year: Some(year),
                });
                continue;
            }

            if let Some(year) = year_from_folder_path(&track.file_path) {
                filled_folder_path += 1;
                to_stage.push(TrackChange {
                    track_id: track.id.clone(),
                    genre: None,
                    comments: None,
                    rating: None,
                    color: None,
                    label: None,
                    year: Some(year),
                });
                continue;
            }

            // Fall back to Discogs enrichment.
            if let Some(year) = discogs_year_for_track(&store_conn, track) {
                filled_discogs += 1;
                to_stage.push(TrackChange {
                    track_id: track.id.clone(),
                    genre: None,
                    comments: None,
                    rating: None,
                    color: None,
                    label: None,
                    year: Some(year),
                });
                continue;
            }

            // Fall back to Beatport enrichment.
            if let Some(year) = beatport_year_for_track(&store_conn, track) {
                filled_beatport += 1;
                to_stage.push(TrackChange {
                    track_id: track.id.clone(),
                    genre: None,
                    comments: None,
                    rating: None,
                    color: None,
                    label: None,
                    year: Some(year),
                });
                continue;
            }

            // Fall back to MusicBrainz enrichment.
            if let Some(year) = musicbrainz_year_for_track(&store_conn, track) {
                filled_musicbrainz += 1;
                to_stage.push(TrackChange {
                    track_id: track.id.clone(),
                    genre: None,
                    comments: None,
                    rating: None,
                    color: None,
                    label: None,
                    year: Some(year),
                });
                continue;
            }

            // Fall back to Bandcamp enrichment.
            if let Some(year) = bandcamp_year_for_track(&store_conn, track) {
                filled_bandcamp += 1;
                to_stage.push(TrackChange {
                    track_id: track.id.clone(),
                    genre: None,
                    comments: None,
                    rating: None,
                    color: None,
                    label: None,
                    year: Some(year),
                });
                continue;
            }

            remaining_year_zero.push(serde_json::json!({
                "track_id": track.id,
                "artist": track.artist,
                "title": track.title,
                "file_path": track.file_path,
            }));
        } else {
            // Track already has a year — compare against Discogs for conflicts.
            let discogs_year = discogs_year_for_track(&store_conn, track);
            if let Some(enrich_year) = discogs_year {
                if track.year == enrich_year {
                    already_set += 1;
                } else {
                    conflicts.push(serde_json::json!({
                        "track_id": track.id,
                        "artist": track.artist,
                        "title": track.title,
                        "current_year": track.year,
                        "enrichment_year": enrich_year,
                    }));
                }
            } else {
                already_set += 1;
            }
        }
    }

    drop(store_conn);

    let filled =
        filled_file_tags + filled_folder_path + filled_discogs + filled_beatport + filled_musicbrainz + filled_bandcamp;

    let staged_count = if !dry_run && !to_stage.is_empty() {
        let (staged, _) = server.state.changes.stage(to_stage);
        staged
    } else {
        0
    };

    let pending = server.state.changes.pending_ids().len();

    let result = serde_json::json!({
        "summary": {
            "total_scanned": tracks.len(),
            "filled": filled,
            "filled_by_source": {
                "file_tags": filled_file_tags,
                "folder_path": filled_folder_path,
                "discogs": filled_discogs,
                "beatport": filled_beatport,
                "musicbrainz": filled_musicbrainz,
                "bandcamp": filled_bandcamp,
            },
            "already_set": already_set,
            "conflicts": conflicts.len(),
            "remaining_year_zero": remaining_year_zero.len(),
        },
        "staged": staged_count,
        "total_pending": pending,
        "dry_run": dry_run,
        "conflicts": conflicts,
        "remaining_year_zero": remaining_year_zero,
    });

    let json =
        serde_json::to_string_pretty(&result).map_err(|e| mcp_internal_error(format!("{e}")))?;
    Ok(CallToolResult::success(vec![Content::text(json)]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_year_str_plain() {
        assert_eq!(parse_year_str("2019"), Some(2019));
        assert_eq!(parse_year_str(" 1992 "), Some(1992));
    }

    #[test]
    fn parse_year_str_iso_date() {
        assert_eq!(parse_year_str("2019-01-15"), Some(2019));
        assert_eq!(parse_year_str("2024-12"), Some(2024));
    }

    #[test]
    fn parse_year_str_beatport_datetime() {
        assert_eq!(parse_year_str("2010-12-06T00:00:00"), Some(2010));
        assert_eq!(parse_year_str("2023-03-17T00:00:00"), Some(2023));
    }

    #[test]
    fn parse_year_str_rejects_invalid() {
        assert_eq!(parse_year_str(""), None);
        assert_eq!(parse_year_str("abc"), None);
        assert_eq!(parse_year_str("1899"), None);
        assert_eq!(parse_year_str("2100"), None);
    }

    #[test]
    fn folder_path_extracts_year_suffix() {
        assert_eq!(
            year_from_folder_path("/music/Artist/Album (2019)/01 Track.wav"),
            Some(2019)
        );
        assert_eq!(
            year_from_folder_path("/music/Artist/Album (1993)/track.flac"),
            Some(1993)
        );
    }

    #[test]
    fn folder_path_no_year() {
        assert_eq!(year_from_folder_path("/music/play/play25/Track.wav"), None);
        assert_eq!(year_from_folder_path("/music/Artist/Album/track.wav"), None);
    }

    #[test]
    fn folder_path_rejects_invalid_year() {
        assert_eq!(
            year_from_folder_path("/music/Artist/Album (1899)/track.wav"),
            None
        );
        assert_eq!(
            year_from_folder_path("/music/Artist/Album (2100)/track.wav"),
            None
        );
    }

    #[test]
    fn folder_path_ignores_non_suffix_parens() {
        // Year must be at the end in parentheses
        assert_eq!(
            year_from_folder_path("/music/Artist/(2019) Album/track.wav"),
            None
        );
    }
}

/// Look up the Discogs enrichment year for a track. Returns `None` if no
/// enrichment is cached or the cached entry has no valid year.
fn discogs_year_for_track(
    store_conn: &rusqlite::Connection,
    track: &crate::types::Track,
) -> Option<i32> {
    let norm_artist = normalize::normalize_for_matching(&track.artist);
    let norm_title = normalize::normalize_for_matching(&track.title);
    let norm_album = normalize::normalize_for_matching(&track.album);
    let norm_album = (!norm_album.is_empty()).then_some(norm_album);

    let discogs_cache = store::get_enrichment(
        store_conn,
        "discogs",
        &norm_artist,
        &norm_title,
        norm_album.as_deref(),
    )
    .unwrap_or_else(|e| {
        tracing::warn!(provider = "discogs", artist = %norm_artist, title = %norm_title,
            "get_enrichment failed: {e}");
        None
    })?;

    let discogs_val = classify_handler::parse_response_json(Some(&discogs_cache));
    discogs_val
        .as_ref()
        .and_then(|v| v.get("year"))
        .and_then(|v| match v {
            serde_json::Value::Number(n) => n.as_i64().map(|n| n.to_string()),
            serde_json::Value::String(s) => Some(s.clone()),
            _ => None,
        })
        .and_then(|s| parse_year_str(&s))
}

/// Look up the Beatport enrichment year for a track. Returns `None` if no
/// enrichment is cached or the cached entry has no release date.
/// Beatport stores `release_date` as `"YYYY-MM-DDT00:00:00"`.
fn beatport_year_for_track(
    store_conn: &rusqlite::Connection,
    track: &crate::types::Track,
) -> Option<i32> {
    let norm_artist = normalize::normalize_for_matching(&track.artist);
    let norm_title = normalize::normalize_for_matching(&track.title);

    let beatport_cache = store::get_enrichment(store_conn, "beatport", &norm_artist, &norm_title, None)
        .unwrap_or_else(|e| {
            tracing::warn!(provider = "beatport", artist = %norm_artist, title = %norm_title,
                "get_enrichment failed: {e}");
            None
        })?;

    let beatport_val = classify_handler::parse_response_json(Some(&beatport_cache));
    beatport_val
        .as_ref()
        .and_then(|v| v.get("release_date"))
        .and_then(|v| v.as_str())
        .and_then(|s| parse_year_str(s))
}

/// Look up the Bandcamp enrichment year for a track. Returns `None` if no
/// enrichment is cached or the cached entry has no release date.
fn bandcamp_year_for_track(
    store_conn: &rusqlite::Connection,
    track: &crate::types::Track,
) -> Option<i32> {
    let norm_artist = normalize::normalize_for_matching(&track.artist);
    let norm_title = normalize::normalize_for_matching(&track.title);

    let bc_cache = store::get_enrichment(store_conn, "bandcamp", &norm_artist, &norm_title, None)
        .unwrap_or_else(|e| {
            tracing::warn!(provider = "bandcamp", artist = %norm_artist, title = %norm_title,
                "get_enrichment failed: {e}");
            None
        })?;

    let bc_val = classify_handler::parse_response_json(Some(&bc_cache));
    bc_val
        .as_ref()
        .and_then(|v| v.get("release_date"))
        .and_then(|v| v.as_str())
        .and_then(|s| parse_year_str(s))
}

/// Look up the MusicBrainz enrichment year for a track. Returns `None` if no
/// enrichment is cached or the cached entry has no first-release-date.
fn musicbrainz_year_for_track(
    store_conn: &rusqlite::Connection,
    track: &crate::types::Track,
) -> Option<i32> {
    let norm_artist = normalize::normalize_for_matching(&track.artist);
    let norm_title = normalize::normalize_for_matching(&track.title);

    let mb_cache = store::get_enrichment(store_conn, "musicbrainz", &norm_artist, &norm_title, None)
        .unwrap_or_else(|e| {
            tracing::warn!(provider = "musicbrainz", artist = %norm_artist, title = %norm_title,
                "get_enrichment failed: {e}");
            None
        })?;

    let mb_val = classify_handler::parse_response_json(Some(&mb_cache));
    mb_val
        .as_ref()
        .and_then(|v| v.get("first_release_date"))
        .and_then(|v| v.as_str())
        .and_then(|s| parse_year_str(s))
}
