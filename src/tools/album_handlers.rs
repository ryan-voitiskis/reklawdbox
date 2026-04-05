use std::path::Path;

use rmcp::ErrorData as McpError;
use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;

use super::*;
use crate::db;
use crate::normalize;
use crate::store;
use crate::tags;
use crate::types::TrackChange;

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct BackfillAlbumsParams {
    #[schemars(description = "Preview changes without staging (default false)")]
    pub dry_run: Option<bool>,
    #[schemars(
        description = "Automatically enrich uncached tracks via Bandcamp before backfilling (default false)."
    )]
    pub auto_enrich: Option<bool>,
}

/// Known edition/qualifier words — case-insensitive match on parenthetical content.
const QUALIFIER_WORDS: &[&str] = &[
    "edition",
    "remaster",
    "remastered",
    "deluxe",
    "special",
    "limited",
    "expanded",
    "anniversary",
    "reissue",
    "soundtrack",
    "bonus",
    "version",
    "ost",
    "japanese",
];

/// Extract album name from a `(YYYY)` suffixed parent directory.
/// Strips the year suffix and any trailing edition qualifiers.
fn album_from_folder_path(file_path: &str) -> Option<String> {
    let path = Path::new(file_path);
    let parent = path.parent()?;
    let dir_name = parent.file_name()?.to_str()?;
    let trimmed = dir_name.trim_end();

    if trimmed.len() < 6 || trimmed.as_bytes()[trimmed.len() - 1] != b')' {
        return None;
    }
    let open = trimmed.rfind('(')?;
    let inside = &trimmed[open + 1..trimmed.len() - 1];
    let prefix = inside.get(..4)?;
    if !prefix.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let year: i32 = prefix.parse().ok()?;
    if !(1900..=2099).contains(&year) {
        return None;
    }

    let mut name = trimmed[..open].trim_end().to_string();

    // Repeatedly strip trailing (...) that contain qualifier words
    loop {
        let current = name.trim_end();
        if current.is_empty() || current.as_bytes()[current.len() - 1] != b')' {
            break;
        }
        let Some(paren_open) = current.rfind('(') else {
            break;
        };
        let paren_content = &current[paren_open + 1..current.len() - 1];
        let lower = paren_content.to_lowercase();
        let is_qualifier = QUALIFIER_WORDS.iter().any(|word| lower.contains(word));
        if is_qualifier {
            name = current[..paren_open].trim_end().to_string();
        } else {
            break;
        }
    }

    let name = name.trim().to_string();
    if name.is_empty() {
        return None;
    }
    Some(name)
}

fn album_from_file_tags(file_path: &str) -> Option<String> {
    let fields = ["album".to_string()];
    let result = tags::read_file_tags(Path::new(file_path), Some(&fields), false);
    let album_str = match result {
        tags::FileReadResult::Single { ref tags, .. } => tags.get("album")?.clone(),
        tags::FileReadResult::Wav {
            ref id3v2,
            ref riff_info,
            ..
        } => id3v2
            .get("album")
            .or_else(|| riff_info.get("album"))?
            .clone(),
        tags::FileReadResult::Error { .. } => return None,
    };
    album_str
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn is_noise_album(candidate: &str, track_title: &str, artist: &str) -> bool {
    let norm_candidate = normalize::normalize_for_matching(candidate);
    let norm_title = normalize::normalize_for_matching(track_title);
    let norm_artist = normalize::normalize_for_matching(artist);

    norm_candidate == norm_title || norm_candidate == norm_artist
}

#[derive(Default)]
struct BackfillAlbumsScanResult {
    filled: usize,
    already_set: usize,
    no_source: usize,
    skipped_noise: usize,
    no_bandcamp: usize,
    to_stage: Vec<TrackChange>,
    filled_by_source: FilledBySource,
    /// Tracks with no Bandcamp cache and no album from any source.
    uncached_bandcamp: Vec<(String, String, String, String)>,
}

#[derive(Default)]
struct FilledBySource {
    file_tags: usize,
    folder_path: usize,
    bandcamp: usize,
    discogs: usize,
}

fn extract_album(entry: Option<&store::EnrichmentCacheEntry>) -> Option<String> {
    classify_handler::parse_response_json(entry)
        .as_ref()
        .and_then(|v| {
            // Bandcamp: "album" field; Discogs: "title" field (release name)
            v.get("album")
                .or_else(|| v.get("title"))
                .and_then(|v| v.as_str())
        })
        .filter(|a| !a.is_empty())
        .map(|a| a.trim().to_string())
}

fn scan_albums(
    store_conn: &rusqlite::Connection,
    tracks: &[crate::types::Track],
) -> BackfillAlbumsScanResult {
    let mut r = BackfillAlbumsScanResult::default();

    // Pre-compute normalized keys for all tracks.
    let norm_keys: Vec<(String, String)> = tracks
        .iter()
        .map(|t| {
            let a = normalize::normalize_for_matching(&t.artist);
            let ti = normalize::normalize_for_matching(&t.title);
            (a, ti)
        })
        .collect();

    // Build batch keys for both providers × all tracks.
    let mut enrich_keys: Vec<(&str, &str, &str, &str)> = Vec::with_capacity(tracks.len() * 2);
    for (a, t) in &norm_keys {
        enrich_keys.push(("bandcamp", a, t, ""));
        enrich_keys.push(("discogs", a, t, ""));
    }

    // Single batch load — replaces 2N individual queries.
    let cache_map = store::batch_get_enrichment(store_conn, &enrich_keys).unwrap_or_else(|e| {
        tracing::warn!("batch enrichment load failed: {e}");
        std::collections::HashMap::new()
    });

    for (track, (norm_artist, norm_title)) in tracks.iter().zip(&norm_keys) {
        if !track.album.is_empty() {
            r.already_set += 1;
            continue;
        }

        let bc_key = (
            "bandcamp".to_string(),
            norm_artist.clone(),
            norm_title.clone(),
            String::new(),
        );
        let dc_key = (
            "discogs".to_string(),
            norm_artist.clone(),
            norm_title.clone(),
            String::new(),
        );

        let bc_entry = cache_map.get(&bc_key);
        let dc_entry = cache_map.get(&dc_key);

        // Source cascade: file tags → folder path → bandcamp → discogs
        let mut source = None;

        if let Some(album) = album_from_file_tags(&track.file_path) {
            if !is_noise_album(&album, &track.title, &track.artist) {
                source = Some(("file_tags", album));
            } else {
                r.skipped_noise += 1;
            }
        }

        if source.is_none()
            && let Some(album) = album_from_folder_path(&track.file_path)
        {
            if !is_noise_album(&album, &track.title, &track.artist) {
                source = Some(("folder_path", album));
            } else {
                r.skipped_noise += 1;
            }
        }

        if source.is_none() {
            if let Some(album) = extract_album(bc_entry) {
                if !is_noise_album(&album, &track.title, &track.artist) {
                    source = Some(("bandcamp", album));
                } else {
                    r.skipped_noise += 1;
                }
            }

            if source.is_none()
                && let Some(album) = extract_album(dc_entry)
            {
                if !is_noise_album(&album, &track.title, &track.artist) {
                    source = Some(("discogs", album));
                } else {
                    r.skipped_noise += 1;
                }
            }

            if source.is_none() && bc_entry.is_none() {
                r.no_bandcamp += 1;
                r.uncached_bandcamp.push((
                    norm_artist.clone(),
                    norm_title.clone(),
                    track.artist.clone(),
                    track.title.clone(),
                ));
            }
        }

        match source {
            Some((src, album)) => {
                r.filled += 1;
                match src {
                    "file_tags" => r.filled_by_source.file_tags += 1,
                    "folder_path" => r.filled_by_source.folder_path += 1,
                    "bandcamp" => r.filled_by_source.bandcamp += 1,
                    "discogs" => r.filled_by_source.discogs += 1,
                    _ => {}
                }
                r.to_stage.push(TrackChange {
                    track_id: track.id.clone(),
                    album: Some(album),
                    ..Default::default()
                });
            }
            None => {
                r.no_source += 1;
            }
        }
    }

    r
}

pub(super) async fn handle_backfill_albums(
    server: &ReklawdboxServer,
    params: BackfillAlbumsParams,
) -> Result<CallToolResult, McpError> {
    let dry_run = params.dry_run.unwrap_or(false);
    let auto_enrich = params.auto_enrich.unwrap_or(false);

    let (tracks, mut scan) = {
        let rb_conn = server.rekordbox_conn()?;
        let search_params = db::SearchParams {
            exclude_samples: true,
            ..Default::default()
        };
        let tracks = db::search_tracks_unbounded(&rb_conn, &search_params).map_err(db_error)?;
        drop(rb_conn);

        let store_conn = server.cache_store_conn()?;
        let scan = scan_albums(&store_conn, &tracks);
        drop(store_conn);
        (tracks, scan)
    };

    let mut auto_enriched = 0usize;

    if auto_enrich && !scan.uncached_bandcamp.is_empty() {
        let to_enrich: Vec<_> = std::mem::take(&mut scan.uncached_bandcamp);
        let total = to_enrich.len();
        tracing::info!(
            count = total,
            "auto_enrich: fetching Bandcamp for uncached album tracks"
        );

        use super::enrichment_cache::HasScore;

        let store_path = server.cache_store_path();

        // Ensure the DB exists and is migrated before spawning the writer.
        {
            let _conn = server.cache_store_conn()?;
        }

        let (cache_tx, mut cache_rx) =
            tokio::sync::mpsc::channel::<(String, String, Option<String>, Option<String>)>(16);
        let writer_store_path = store_path.clone();
        let writer_handle = tokio::task::spawn_blocking(move || {
            let conn = match store::open(&writer_store_path) {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!("album auto-enrich writer: failed to open store: {e}");
                    return;
                }
            };
            while let Some((norm_artist, norm_title, quality, json)) = cache_rx.blocking_recv() {
                if let Err(e) = store::set_enrichment(
                    &conn,
                    "bandcamp",
                    &norm_artist,
                    &norm_title,
                    None,
                    quality.as_deref(),
                    json.as_deref(),
                ) {
                    tracing::error!(
                        "album auto-enrich writer: failed to write bandcamp for {norm_artist}/{norm_title}: {e}"
                    );
                }
            }
        });

        let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(4));
        let mut handles = Vec::with_capacity(total);

        for (norm_artist, norm_title, raw_artist, raw_title) in to_enrich {
            let permit = sem
                .clone()
                .acquire_owned()
                .await
                .map_err(|e| mcp_internal_error(format!("Semaphore error: {e}")))?;

            let server = server.clone();
            let cache_tx = cache_tx.clone();

            handles.push(tokio::spawn(async move {
                let _permit = permit;
                let enriched: usize;
                match crate::bandcamp::lookup(&server.state.http, &raw_artist, &raw_title).await {
                    Ok(result) => {
                        let (quality, json) = match &result {
                            Some(r) => {
                                let q = if r.score() >= 90 { "exact" } else { "fuzzy" };
                                let j = serde_json::to_string(r).ok();
                                (Some(q.to_string()), j)
                            }
                            None => (Some("none".to_string()), None),
                        };
                        enriched = if result.is_some() { 1 } else { 0 };
                        let _ = cache_tx
                            .send((norm_artist, norm_title, quality, json))
                            .await;
                    }
                    Err(e) => {
                        tracing::warn!(
                            artist = raw_artist.as_str(),
                            "Bandcamp auto-enrich failed: {e}"
                        );
                        enriched = 0;
                    }
                }
                enriched
            }));
        }

        drop(cache_tx);

        for handle in handles {
            match handle.await {
                Ok(n) => auto_enriched += n,
                Err(e) => tracing::warn!("album auto-enrich task panicked: {e}"),
            }
        }

        if let Err(e) = writer_handle.await {
            tracing::warn!("album backfill cache writer task failed: {e}");
        }

        let store_conn = server.cache_store_conn()?;
        scan = scan_albums(&store_conn, &tracks);
        drop(store_conn);
    }

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
            "filled": scan.filled,
            "already_set": scan.already_set,
            "no_source": scan.no_source,
            "skipped_noise": scan.skipped_noise,
            "filled_by_source": {
                "file_tags": scan.filled_by_source.file_tags,
                "folder_path": scan.filled_by_source.folder_path,
                "bandcamp": scan.filled_by_source.bandcamp,
                "discogs": scan.filled_by_source.discogs,
            },
        },
        "staged": staged_count,
        "total_pending": pending,
        "dry_run": dry_run,
    });

    if auto_enrich {
        result.as_object_mut().unwrap().insert(
            "auto_enriched".to_string(),
            serde_json::json!(auto_enriched),
        );
    }

    ok_json(&result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn album_from_folder_basic() {
        assert_eq!(
            album_from_folder_path("/music/Artist/Album Name (2019)/01 Track.wav"),
            Some("Album Name".to_string())
        );
    }

    #[test]
    fn album_from_folder_strips_edition() {
        assert_eq!(
            album_from_folder_path("/music/The XX/Coexist (Japanese Edition) (2012)/track.flac"),
            Some("Coexist".to_string())
        );
    }

    #[test]
    fn album_from_folder_strips_soundtrack() {
        assert_eq!(
            album_from_folder_path(
                "/music/VA/Hell Or High Water (Original Motion Picture Soundtrack) (2016)/t.wav"
            ),
            Some("Hell Or High Water".to_string())
        );
    }

    #[test]
    fn album_from_folder_strips_deluxe() {
        assert_eq!(
            album_from_folder_path("/music/XX/I See You (Deluxe Edition) (2017)/track.flac"),
            Some("I See You".to_string())
        );
    }

    #[test]
    fn album_from_folder_no_year() {
        assert_eq!(album_from_folder_path("/music/play/play1/track.wav"), None,);
    }

    #[test]
    fn album_from_folder_preserves_non_qualifier_parens() {
        assert_eq!(
            album_from_folder_path("/music/Artist/Music (Is My Life) (2020)/track.flac"),
            Some("Music (Is My Life)".to_string())
        );
    }

    #[test]
    fn noise_filter() {
        assert!(is_noise_album("Some Track", "Some Track", "Artist"));
        assert!(is_noise_album("Artist Name", "Track", "Artist Name"));
        assert!(!is_noise_album("Album Name", "Track", "Artist"));
    }
}
