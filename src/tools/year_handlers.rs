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
        } => id3v2.get("year").or_else(|| riff_info.get("year"))?.clone(),
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
    if let Some(prefix) = inside.get(..4)
        && prefix.bytes().all(|b| b.is_ascii_digit())
    {
        let year: i32 = prefix.parse().ok()?;
        return (1900..=2099).contains(&year).then_some(year);
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

/// Extract year from a Discogs enrichment cache entry.
fn extract_discogs_year(entry: Option<&store::EnrichmentCacheEntry>) -> Option<i32> {
    let val = classify_handler::parse_response_json(entry)?;
    val.get("year")
        .and_then(|v| match v {
            serde_json::Value::Number(n) => n.as_i64().map(|n| n.to_string()),
            serde_json::Value::String(s) => Some(s.clone()),
            _ => None,
        })
        .and_then(|s| parse_year_str(&s))
}

/// Extract year from a Beatport enrichment cache entry.
fn extract_beatport_year(entry: Option<&store::EnrichmentCacheEntry>) -> Option<i32> {
    let val = classify_handler::parse_response_json(entry)?;
    val.get("release_date")
        .and_then(|v| v.as_str())
        .and_then(parse_year_str)
}

/// Extract year from a MusicBrainz enrichment cache entry.
fn extract_musicbrainz_year(entry: Option<&store::EnrichmentCacheEntry>) -> Option<i32> {
    let val = classify_handler::parse_response_json(entry)?;
    val.get("first_release_date")
        .and_then(|v| v.as_str())
        .and_then(parse_year_str)
}

/// Extract year from a Bandcamp enrichment cache entry.
fn extract_bandcamp_year(entry: Option<&store::EnrichmentCacheEntry>) -> Option<i32> {
    let val = classify_handler::parse_response_json(entry)?;
    val.get("release_date")
        .and_then(|v| v.as_str())
        .and_then(parse_year_str)
}

fn scan_years(
    store_conn: &rusqlite::Connection,
    tracks: &[crate::types::Track],
) -> BackfillYearsScanResult {
    let mut r = BackfillYearsScanResult::default();

    let year_change = |track_id: String, year: i32| TrackChange {
        track_id,
        genre: None,
        comments: None,
        rating: None,
        color: None,
        label: None,
        year: Some(year),
        album: None,
    };

    // Pre-compute normalized keys for all tracks.
    let norm_keys: Vec<(String, String, Option<String>)> = tracks
        .iter()
        .map(|t| {
            let a = normalize::normalize_for_matching(&t.artist);
            let ti = normalize::normalize_for_matching(&t.title);
            let al = normalize::normalize_for_matching(&t.album);
            (a, ti, (!al.is_empty()).then_some(al))
        })
        .collect();

    // Build batch keys for all 4 providers × all tracks.
    let mut enrich_keys: Vec<(&str, &str, &str, &str)> =
        Vec::with_capacity(tracks.len() * 4);
    for (a, t, al) in &norm_keys {
        let album = al.as_deref().unwrap_or("");
        enrich_keys.push(("discogs", a, t, album));
        enrich_keys.push(("beatport", a, t, ""));
        enrich_keys.push(("musicbrainz", a, t, ""));
        enrich_keys.push(("bandcamp", a, t, ""));
    }

    // Single batch load — replaces up to 8N individual queries.
    let cache_map = store::batch_get_enrichment(store_conn, &enrich_keys)
        .unwrap_or_else(|e| {
            tracing::warn!("batch enrichment load failed: {e}");
            std::collections::HashMap::new()
        });

    for (track, (norm_artist, norm_title, norm_album)) in tracks.iter().zip(&norm_keys) {
        let album = norm_album.as_deref().unwrap_or("");

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

            let discogs_key = ("discogs".to_string(), norm_artist.clone(), norm_title.clone(), album.to_string());
            let bp_key = ("beatport".to_string(), norm_artist.clone(), norm_title.clone(), String::new());
            let mb_key = ("musicbrainz".to_string(), norm_artist.clone(), norm_title.clone(), String::new());
            let bc_key = ("bandcamp".to_string(), norm_artist.clone(), norm_title.clone(), String::new());

            let discogs_entry = cache_map.get(&discogs_key);
            let bp_entry = cache_map.get(&bp_key);
            let mb_entry = cache_map.get(&mb_key);
            let bc_entry = cache_map.get(&bc_key);

            if let Some(year) = extract_discogs_year(discogs_entry) {
                r.filled_discogs += 1;
                r.to_stage.push(year_change(track.id.clone(), year));
                continue;
            }
            if let Some(year) = extract_beatport_year(bp_entry) {
                r.filled_beatport += 1;
                r.to_stage.push(year_change(track.id.clone(), year));
                continue;
            }
            if let Some(year) = extract_musicbrainz_year(mb_entry) {
                r.filled_musicbrainz += 1;
                r.to_stage.push(year_change(track.id.clone(), year));
                continue;
            }
            if let Some(year) = extract_bandcamp_year(bc_entry) {
                r.filled_bandcamp += 1;
                r.to_stage.push(year_change(track.id.clone(), year));
                continue;
            }

            // Cache-gap tracking — presence in the map means cached (no second query needed).
            if discogs_entry.is_none() {
                r.remaining_no_discogs += 1;
            }
            if bp_entry.is_none() {
                r.remaining_no_beatport += 1;
            }
            if mb_entry.is_none() {
                r.remaining_no_musicbrainz += 1;
                r.uncached_musicbrainz.push((
                    norm_artist.clone(),
                    norm_title.clone(),
                    track.artist.clone(),
                    track.title.clone(),
                ));
            }
            if bc_entry.is_none() {
                r.remaining_no_bandcamp += 1;
                r.uncached_bandcamp.push((
                    norm_artist.clone(),
                    norm_title.clone(),
                    track.artist.clone(),
                    track.title.clone(),
                ));
            }

            r.remaining_year_zero.push(serde_json::json!({
                "track_id": track.id,
                "artist": track.artist,
                "title": track.title,
            }));
        } else {
            let discogs_key = ("discogs".to_string(), norm_artist.clone(), norm_title.clone(), album.to_string());
            let discogs_year = extract_discogs_year(cache_map.get(&discogs_key));
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
        let tracks = db::search_tracks_unbounded(&rb_conn, &search_params).map_err(db_error)?;
        drop(rb_conn);

        let store_conn = server.cache_store_conn()?;
        let scan = scan_years(&store_conn, &tracks);
        drop(store_conn);
        (tracks, scan)
    };

    let mut auto_enriched = 0usize;

    if auto_enrich && (!scan.uncached_bandcamp.is_empty() || !scan.uncached_musicbrainz.is_empty())
    {
        use super::enrichment_cache::HasScore;

        let bc_tracks: Vec<_> = std::mem::take(&mut scan.uncached_bandcamp);
        let mb_tracks: Vec<_> = std::mem::take(&mut scan.uncached_musicbrainz);
        let total = bc_tracks.len() + mb_tracks.len();
        tracing::info!(
            bandcamp = bc_tracks.len(),
            musicbrainz = mb_tracks.len(),
            "auto_enrich: fetching for {total} uncached year-zero tracks"
        );

        // Shared channel + spawn_blocking writer for cache writes.
        let (cache_tx, mut cache_rx) = tokio::sync::mpsc::channel::<(
            String,
            String,
            String,
            Option<String>,
            Option<String>,
        )>(32);

        let writer_store_path = server.cache_store_path();
        let writer_handle = tokio::task::spawn_blocking(move || {
            let conn = match store::open(&writer_store_path) {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!("Year backfill cache writer: failed to open store: {e}");
                    return;
                }
            };
            while let Some((provider, norm_artist, norm_title, match_quality, response_json)) =
                cache_rx.blocking_recv()
            {
                if let Err(e) = store::set_enrichment(
                    &conn,
                    &provider,
                    &norm_artist,
                    &norm_title,
                    None,
                    match_quality.as_deref(),
                    response_json.as_deref(),
                ) {
                    tracing::error!(
                        "Year backfill cache writer: failed to write {provider} for {norm_artist}/{norm_title}: {e}"
                    );
                }
            }
        });

        // Two concurrent dispatch futures — one per provider.
        let bc_future = {
            let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(4));
            let mut handles = Vec::with_capacity(bc_tracks.len());

            for (norm_artist, norm_title, raw_artist, raw_title) in bc_tracks {
                let permit = sem
                    .clone()
                    .acquire_owned()
                    .await
                    .map_err(|e| mcp_internal_error(format!("Semaphore error: {e}")))?;
                let server = server.clone();
                let cache_tx = cache_tx.clone();

                handles.push(tokio::spawn(async move {
                    let enriched = match lookup_bandcamp_remote(&server, &raw_artist, &raw_title)
                        .await
                    {
                        Ok(Some(r)) => {
                            let json_str = match serde_json::to_string(&r) {
                                Ok(j) => j,
                                Err(e) => {
                                    tracing::warn!(
                                        provider = "bandcamp",
                                        artist = norm_artist.as_str(),
                                        "serialization failed: {e}"
                                    );
                                    drop(permit);
                                    return 1usize;
                                }
                            };
                            let quality = if r.score() >= 90 {
                                "exact".to_string()
                            } else {
                                "fuzzy".to_string()
                            };
                            let _ = cache_tx
                                .send((
                                    "bandcamp".to_string(),
                                    norm_artist,
                                    norm_title,
                                    Some(quality),
                                    Some(json_str),
                                ))
                                .await;
                            1usize
                        }
                        Ok(None) => {
                            let _ = cache_tx
                                .send((
                                    "bandcamp".to_string(),
                                    norm_artist,
                                    norm_title,
                                    Some("none".to_string()),
                                    None,
                                ))
                                .await;
                            0usize
                        }
                        Err(e) => {
                            tracing::warn!(
                                artist = raw_artist.as_str(),
                                "Bandcamp auto-enrich failed: {e}"
                            );
                            0usize
                        }
                    };
                    drop(permit);
                    enriched
                }));
            }

            async move {
                let mut sum = 0usize;
                for handle in handles {
                    match handle.await {
                        Ok(n) => sum += n,
                        Err(e) => tracing::warn!("Bandcamp task panicked: {e}"),
                    }
                }
                sum
            }
        };

        let mb_future = {
            let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(4));
            let mut handles = Vec::with_capacity(mb_tracks.len());

            for (norm_artist, norm_title, raw_artist, raw_title) in mb_tracks {
                let permit = sem
                    .clone()
                    .acquire_owned()
                    .await
                    .map_err(|e| mcp_internal_error(format!("Semaphore error: {e}")))?;
                let server = server.clone();
                let cache_tx = cache_tx.clone();

                handles.push(tokio::spawn(async move {
                    let enriched =
                        match lookup_musicbrainz_remote(&server, &raw_artist, &raw_title).await {
                            Ok(Some(r)) => {
                                let json_str = match serde_json::to_string(&r) {
                                    Ok(j) => j,
                                    Err(e) => {
                                        tracing::warn!(
                                            provider = "musicbrainz",
                                            artist = norm_artist.as_str(),
                                            "serialization failed: {e}"
                                        );
                                        drop(permit);
                                        return 1usize;
                                    }
                                };
                                let quality = if r.score() >= 90 {
                                    "exact".to_string()
                                } else {
                                    "fuzzy".to_string()
                                };
                                let _ = cache_tx
                                    .send((
                                        "musicbrainz".to_string(),
                                        norm_artist,
                                        norm_title,
                                        Some(quality),
                                        Some(json_str),
                                    ))
                                    .await;
                                1usize
                            }
                            Ok(None) => {
                                let _ = cache_tx
                                    .send((
                                        "musicbrainz".to_string(),
                                        norm_artist,
                                        norm_title,
                                        Some("none".to_string()),
                                        None,
                                    ))
                                    .await;
                                0usize
                            }
                            Err(e) => {
                                tracing::warn!(
                                    artist = raw_artist.as_str(),
                                    "MusicBrainz auto-enrich failed: {e}"
                                );
                                0usize
                            }
                        };
                    drop(permit);
                    enriched
                }));
            }

            async move {
                let mut sum = 0usize;
                for handle in handles {
                    match handle.await {
                        Ok(n) => sum += n,
                        Err(e) => tracing::warn!("MusicBrainz task panicked: {e}"),
                    }
                }
                sum
            }
        };

        // Run both provider dispatches concurrently.
        let (bc_enriched, mb_enriched) = tokio::join!(bc_future, mb_future);
        auto_enriched = bc_enriched + mb_enriched;

        // Close channel so writer drains and exits.
        drop(cache_tx);
        if let Err(e) = writer_handle.await {
            tracing::error!("Year backfill cache writer task failed: {e}");
        }

        // Re-scan with updated cache
        let store_conn = server.cache_store_conn()?;
        scan = scan_years(&store_conn, &tracks);
        drop(store_conn);
    }

    let filled = scan.filled_file_tags
        + scan.filled_folder_path
        + scan.filled_discogs
        + scan.filled_beatport
        + scan.filled_musicbrainz
        + scan.filled_bandcamp;

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

    let json = serde_json::to_string(&result).map_err(|e| mcp_internal_error(format!("{e}")))?;
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
