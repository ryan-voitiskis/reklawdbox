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
    #[schemars(
        description = "Automatically enrich remaining year-zero tracks via Bandcamp and MusicBrainz before re-scanning (default false). Fetches data for tracks missing from enrichment caches, then re-runs the year cascade."
    )]
    pub auto_enrich: Option<bool>,
}

/// Cache a lookup result (hit or miss). Returns 1 if the result had data, 0 if miss.
fn cache_lookup_result<T: serde::Serialize + HasScore>(
    server: &ReklawdboxServer,
    provider: &str,
    norm_artist: &str,
    norm_title: &str,
    result: Option<&T>,
) -> Result<usize, McpError> {
    let store_conn = server.cache_store_conn()?;
    match result {
        Some(r) => {
            let json = serde_json::to_string(r).ok();
            let quality = if r.score() >= 90 { "exact" } else { "fuzzy" };
            let _ = store::set_enrichment(
                &store_conn, provider, norm_artist, norm_title, None,
                Some(quality), json.as_deref(),
            );
            Ok(1)
        }
        None => {
            let _ = store::set_enrichment(
                &store_conn, provider, norm_artist, norm_title, None,
                Some("none"), None,
            );
            Ok(0)
        }
    }
}

trait HasScore {
    fn score(&self) -> i32;
}

impl HasScore for crate::bandcamp::BandcampResult {
    fn score(&self) -> i32 { self.score }
}

impl HasScore for crate::musicbrainz::MusicBrainzResult {
    fn score(&self) -> i32 { self.score }
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

#[derive(Default)]
struct BackfillYearsScanResult {
    filled_file_tags: usize,
    filled_folder_path: usize,
    filled_discogs: usize,
    filled_beatport: usize,
    filled_musicbrainz: usize,
    filled_bandcamp: usize,
    already_set: usize,
    conflicts: Vec<serde_json::Value>,
    remaining_year_zero: Vec<serde_json::Value>,
    remaining_no_discogs: usize,
    remaining_no_beatport: usize,
    remaining_no_musicbrainz: usize,
    remaining_no_bandcamp: usize,
    to_stage: Vec<TrackChange>,
    /// Tracks needing Bandcamp enrichment: (norm_artist, norm_title, raw_artist, raw_title).
    uncached_bandcamp: Vec<(String, String, String, String)>,
    /// Tracks needing MusicBrainz enrichment: (norm_artist, norm_title, raw_artist, raw_title).
    uncached_musicbrainz: Vec<(String, String, String, String)>,
}

fn scan_years(
    store_conn: &rusqlite::Connection,
    tracks: &[crate::types::Track],
) -> BackfillYearsScanResult {
    let mut r = BackfillYearsScanResult::default();

    let year_change = |track_id: String, year: i32| TrackChange {
        track_id, genre: None, comments: None,
        rating: None, color: None, label: None, year: Some(year),
    };

    for track in tracks {
        if track.year == 0 {
            // Priority cascade: file tags → folder path → Discogs → Beatport → MusicBrainz → Bandcamp.
            if let Some(year) = year_from_file_tags(&track.file_path) {
                r.filled_file_tags += 1;
                r.to_stage.push(year_change(track.id.clone(), year));
                continue;
            }
            if let Some(year) = year_from_folder_path(&track.file_path) {
                r.filled_folder_path += 1;
                r.to_stage.push(year_change(track.id.clone(), year));
                continue;
            }
            if let Some(year) = discogs_year_for_track(store_conn, track) {
                r.filled_discogs += 1;
                r.to_stage.push(year_change(track.id.clone(), year));
                continue;
            }
            if let Some(year) = beatport_year_for_track(store_conn, track) {
                r.filled_beatport += 1;
                r.to_stage.push(year_change(track.id.clone(), year));
                continue;
            }
            if let Some(year) = musicbrainz_year_for_track(store_conn, track) {
                r.filled_musicbrainz += 1;
                r.to_stage.push(year_change(track.id.clone(), year));
                continue;
            }
            if let Some(year) = bandcamp_year_for_track(store_conn, track) {
                r.filled_bandcamp += 1;
                r.to_stage.push(year_change(track.id.clone(), year));
                continue;
            }

            // Track provider cache gaps for remaining year-zero tracks.
            let norm_artist = normalize::normalize_for_matching(&track.artist);
            let norm_title = normalize::normalize_for_matching(&track.title);
            let has_discogs = store::get_enrichment(store_conn, "discogs", &norm_artist, &norm_title, None)
                .unwrap_or(None).is_some();
            let has_beatport = store::get_enrichment(store_conn, "beatport", &norm_artist, &norm_title, None)
                .unwrap_or(None).is_some();
            let has_musicbrainz = store::get_enrichment(store_conn, "musicbrainz", &norm_artist, &norm_title, None)
                .unwrap_or(None).is_some();
            let has_bandcamp = store::get_enrichment(store_conn, "bandcamp", &norm_artist, &norm_title, None)
                .unwrap_or(None).is_some();
            if !has_discogs { r.remaining_no_discogs += 1; }
            if !has_beatport { r.remaining_no_beatport += 1; }
            if !has_musicbrainz {
                r.remaining_no_musicbrainz += 1;
                r.uncached_musicbrainz.push((norm_artist.clone(), norm_title.clone(), track.artist.clone(), track.title.clone()));
            }
            if !has_bandcamp {
                r.remaining_no_bandcamp += 1;
                r.uncached_bandcamp.push((norm_artist.clone(), norm_title.clone(), track.artist.clone(), track.title.clone()));
            }

            r.remaining_year_zero.push(serde_json::json!({
                "track_id": track.id,
                "artist": track.artist,
                "title": track.title,
                "file_path": track.file_path,
            }));
        } else {
            let discogs_year = discogs_year_for_track(store_conn, track);
            if let Some(enrich_year) = discogs_year {
                if track.year == enrich_year {
                    r.already_set += 1;
                } else {
                    r.conflicts.push(serde_json::json!({
                        "track_id": track.id,
                        "artist": track.artist,
                        "title": track.title,
                        "current_year": track.year,
                        "enrichment_year": enrich_year,
                    }));
                }
            } else {
                r.already_set += 1;
            }
        }
    }

    r
}

pub(super) async fn handle_backfill_years(
    server: &ReklawdboxServer,
    params: BackfillYearsParams,
) -> Result<CallToolResult, McpError> {
    let dry_run = params.dry_run.unwrap_or(false);
    let auto_enrich = params.auto_enrich.unwrap_or(false);

    // Synchronous DB + cache work in a block to drop MutexGuard before any .await.
    let (tracks, mut scan) = {
        let rb_conn = server.rekordbox_conn()?;
        let search_params = db::SearchParams {
            exclude_samples: true,
            ..Default::default()
        };
        let tracks = db::search_tracks_unbounded(&rb_conn, &search_params)
            .map_err(|e| mcp_internal_error(format!("DB error: {e}")))?;
        drop(rb_conn);

        let store_conn = server.cache_store_conn()?;
        let scan = scan_years(&store_conn, &tracks);
        drop(store_conn);
        (tracks, scan)
    };

    let mut auto_enriched = 0usize;

    // Auto-enrich: fetch Bandcamp + MusicBrainz for uncached remaining year-zero tracks.
    if auto_enrich && (!scan.uncached_bandcamp.is_empty() || !scan.uncached_musicbrainz.is_empty()) {
        let bc_tracks: Vec<_> = std::mem::take(&mut scan.uncached_bandcamp);
        let mb_tracks: Vec<_> = std::mem::take(&mut scan.uncached_musicbrainz);
        let total = bc_tracks.len() + mb_tracks.len();
        tracing::info!(bandcamp = bc_tracks.len(), musicbrainz = mb_tracks.len(),
            "auto_enrich: fetching for {total} uncached year-zero tracks");

        // Fetch Bandcamp.
        for (norm_artist, norm_title, raw_artist, raw_title) in &bc_tracks {
            match lookup_bandcamp_remote(server, raw_artist, raw_title).await {
                Ok(result) => {
                    auto_enriched += cache_lookup_result(
                        server, "bandcamp", norm_artist, norm_title, result.as_ref(),
                    )?;
                }
                Err(e) => {
                    tracing::warn!(artist = raw_artist.as_str(), "Bandcamp auto-enrich failed: {e}");
                }
            }
        }

        // Fetch MusicBrainz.
        for (norm_artist, norm_title, raw_artist, raw_title) in &mb_tracks {
            match lookup_musicbrainz_remote(server, raw_artist, raw_title).await {
                Ok(result) => {
                    auto_enriched += cache_lookup_result(
                        server, "musicbrainz", norm_artist, norm_title, result.as_ref(),
                    )?;
                }
                Err(e) => {
                    tracing::warn!(artist = raw_artist.as_str(), "MusicBrainz auto-enrich failed: {e}");
                }
            }
        }

        // Second pass: re-scan with updated cache.
        let store_conn = server.cache_store_conn()?;
        scan = scan_years(&store_conn, &tracks);
        drop(store_conn);
    }

    let filled = scan.filled_file_tags + scan.filled_folder_path + scan.filled_discogs
        + scan.filled_beatport + scan.filled_musicbrainz + scan.filled_bandcamp;

    let staged_count = if !dry_run && !scan.to_stage.is_empty() {
        let (staged, _) = server.state.changes.stage(scan.to_stage);
        staged
    } else {
        0
    };

    let pending = server.state.changes.pending_ids().len();

    let mut result = serde_json::json!({
        "summary": {
            "total_scanned": tracks.len(),
            "filled": filled,
            "filled_by_source": {
                "file_tags": scan.filled_file_tags,
                "folder_path": scan.filled_folder_path,
                "discogs": scan.filled_discogs,
                "beatport": scan.filled_beatport,
                "musicbrainz": scan.filled_musicbrainz,
                "bandcamp": scan.filled_bandcamp,
            },
            "already_set": scan.already_set,
            "conflicts": scan.conflicts.len(),
            "remaining_year_zero": scan.remaining_year_zero.len(),
            "remaining_uncached_providers": {
                "no_discogs": scan.remaining_no_discogs,
                "no_beatport": scan.remaining_no_beatport,
                "no_musicbrainz": scan.remaining_no_musicbrainz,
                "no_bandcamp": scan.remaining_no_bandcamp,
            },
        },
        "staged": staged_count,
        "total_pending": pending,
        "dry_run": dry_run,
        "conflicts": scan.conflicts,
        "remaining_year_zero": scan.remaining_year_zero,
    });

    if auto_enrich {
        result.as_object_mut().unwrap().insert(
            "auto_enriched".to_string(),
            serde_json::json!(auto_enriched),
        );
    }

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
