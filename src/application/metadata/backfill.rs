//! Metadata suggestion cascades and staging orchestration.

use std::path::Path;

use crate::adapters::audio::tags;
use crate::adapters::rekordbox;
use crate::adapters::state;
use crate::application::classification::evidence::parse_response_json;
use crate::domain::classification::taxonomy as genre;
use crate::domain::metadata::{
    COLORS, ChangeManager, TrackChange, canonical_color_name, is_valid_color,
    normalize_for_matching,
};

#[derive(Debug)]
pub(crate) struct StageUpdatesResult {
    pub(crate) staged: usize,
    pub(crate) total_pending: usize,
    pub(crate) warnings: Vec<String>,
}

pub(crate) fn stage_track_updates(
    changes: &ChangeManager,
    mut updates: Vec<TrackChange>,
) -> Result<StageUpdatesResult, String> {
    for update in &updates {
        if update.track_id.trim().is_empty() {
            return Err("track_id must not be empty".to_string());
        }
        if let Some(rating) = update.rating
            && (rating == 0 || rating > 5)
        {
            return Err(format!("rating must be 1-5, got {rating}"));
        }
        if let Some(year) = update.year
            && !(1900..=2099).contains(&year)
        {
            return Err(format!("year must be 1900-2099, got {year}"));
        }
        if let Some(color) = &update.color
            && !is_valid_color(color)
        {
            let valid = COLORS.iter().map(|(name, _)| *name).collect::<Vec<_>>();
            return Err(format!(
                "unknown color '{}'. Valid colors: {}",
                color,
                valid.join(", ")
            ));
        }
    }
    let warnings = updates
        .iter()
        .filter_map(|update| update.genre.as_deref())
        .filter(|value| !genre::is_known_genre(value))
        .map(|value| format!("'{value}' is not in the genre taxonomy"))
        .collect();
    for update in &mut updates {
        if let Some(color) = update.color.take() {
            update.color = Some(
                canonical_color_name(&color)
                    .map(String::from)
                    .unwrap_or(color),
            );
        }
    }
    let (staged, total_pending) = stage_suggestions(changes, updates, false);
    Ok(StageUpdatesResult {
        staged,
        total_pending,
        warnings,
    })
}

pub(crate) fn suggest_normalizations(
    conn: &rusqlite::Connection,
    changes: &ChangeManager,
    min_count: i32,
    stage_aliases: bool,
) -> Result<serde_json::Value, rusqlite::Error> {
    let stats = rekordbox::get_library_stats(conn)?;
    let mut alias_groups = Vec::new();
    let mut unknown_groups = Vec::new();
    let mut canonical_items = Vec::new();
    let mut alias_changes = Vec::new();
    for count in &stats.genres {
        if count.name == "(none)" || count.name.is_empty() || count.count < min_count {
            continue;
        }
        if let Some(canonical) = genre::canonical_genre_from_alias(&count.name) {
            let tracks = rekordbox::get_tracks_by_exact_genre(conn, &count.name, true)?;
            let track_ids = tracks
                .iter()
                .map(|track| track.id.clone())
                .collect::<Vec<_>>();
            if stage_aliases {
                alias_changes.extend(track_ids.iter().map(|track_id| TrackChange {
                    track_id: track_id.clone(),
                    genre: Some(canonical.to_string()),
                    ..Default::default()
                }));
            }
            alias_groups.push((count.name.clone(), canonical, track_ids));
        } else if genre::is_known_genre(&count.name) {
            canonical_items.push(serde_json::json!({
                "genre": count.name,
                "count": count.count,
            }));
        } else {
            let tracks = rekordbox::get_tracks_by_exact_genre(conn, &count.name, true)?;
            unknown_groups.push((
                count.name.clone(),
                tracks
                    .iter()
                    .map(|track| track.id.clone())
                    .collect::<Vec<_>>(),
            ));
        }
    }
    let alias_track_count = alias_groups
        .iter()
        .map(|(_, _, ids)| ids.len())
        .sum::<usize>();
    let unknown_track_count = unknown_groups
        .iter()
        .map(|(_, ids)| ids.len())
        .sum::<usize>();
    let alias_output = alias_groups
        .iter()
        .map(|(current, canonical, ids)| {
            serde_json::json!({
                "from": current,
                "to": canonical,
                "count": ids.len(),
                "track_ids": ids,
            })
        })
        .collect::<Vec<_>>();
    let unknown_output = unknown_groups
        .iter()
        .map(|(current, ids)| {
            serde_json::json!({
                "genre": current,
                "count": ids.len(),
                "track_ids": ids,
            })
        })
        .collect::<Vec<_>>();
    let mut result = serde_json::json!({
        "alias": alias_output,
        "unknown": unknown_output,
        "canonical": canonical_items,
        "summary": {
            "alias_tracks": alias_track_count,
            "unknown_tracks": unknown_track_count,
            "canonical_genres": canonical_items.len(),
        }
    });
    if stage_aliases && !alias_changes.is_empty() {
        let (staged, total_pending) = stage_suggestions(changes, alias_changes, false);
        result["staging"] = serde_json::json!({
            "staged": staged,
            "total_pending": total_pending,
        });
    }
    Ok(result)
}

/// Stage suggestions through the canonical change manager unless this is a preview.
pub(crate) fn stage_suggestions(
    changes: &ChangeManager,
    suggestions: Vec<TrackChange>,
    dry_run: bool,
) -> (usize, usize) {
    if dry_run || suggestions.is_empty() {
        (0, changes.pending_ids().len())
    } else {
        changes.stage(suggestions)
    }
}

pub(crate) fn parse_year_str(s: &str) -> Option<i32> {
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
pub(crate) fn year_from_file_tags(path: &str) -> Option<i32> {
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
pub(crate) fn year_from_folder_path(file_path: &str) -> Option<i32> {
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
pub(crate) struct BackfillYearsScanResult {
    pub(crate) filled_file_tags: usize,
    pub(crate) filled_folder_path: usize,
    pub(crate) filled_discogs: usize,
    pub(crate) filled_beatport: usize,
    pub(crate) filled_musicbrainz: usize,
    pub(crate) filled_bandcamp: usize,
    pub(crate) already_set: usize,
    pub(crate) conflicts: Vec<serde_json::Value>,
    pub(crate) remaining_year_zero: Vec<serde_json::Value>,
    pub(crate) remaining_no_discogs: usize,
    pub(crate) remaining_no_beatport: usize,
    pub(crate) remaining_no_musicbrainz: usize,
    pub(crate) remaining_no_bandcamp: usize,
    pub(crate) to_stage: Vec<TrackChange>,
    /// Tracks needing Bandcamp enrichment: (norm_artist, norm_title, raw_artist, raw_title).
    pub(crate) uncached_bandcamp: Vec<(String, String, String, String)>,
    /// Tracks needing MusicBrainz enrichment: (norm_artist, norm_title, raw_artist, raw_title).
    pub(crate) uncached_musicbrainz: Vec<(String, String, String, String)>,
}

/// Extract year from a Discogs enrichment cache entry.
pub(crate) fn extract_discogs_year(entry: Option<&state::EnrichmentCacheEntry>) -> Option<i32> {
    let val = parse_response_json(entry)?;
    val.get("year")
        .and_then(|v| match v {
            serde_json::Value::Number(n) => n.as_i64().map(|n| n.to_string()),
            serde_json::Value::String(s) => Some(s.clone()),
            _ => None,
        })
        .and_then(|s| parse_year_str(&s))
}

/// Extract year from a Beatport enrichment cache entry.
pub(crate) fn extract_beatport_year(entry: Option<&state::EnrichmentCacheEntry>) -> Option<i32> {
    let val = parse_response_json(entry)?;
    val.get("release_date")
        .and_then(|v| v.as_str())
        .and_then(parse_year_str)
}

/// Extract year from a MusicBrainz enrichment cache entry.
pub(crate) fn extract_musicbrainz_year(entry: Option<&state::EnrichmentCacheEntry>) -> Option<i32> {
    let val = parse_response_json(entry)?;
    val.get("first_release_date")
        .and_then(|v| v.as_str())
        .and_then(parse_year_str)
}

/// Extract year from a Bandcamp enrichment cache entry.
pub(crate) fn extract_bandcamp_year(entry: Option<&state::EnrichmentCacheEntry>) -> Option<i32> {
    let val = parse_response_json(entry)?;
    val.get("release_date")
        .and_then(|v| v.as_str())
        .and_then(parse_year_str)
}

pub(crate) fn scan_years(
    store_conn: &rusqlite::Connection,
    tracks: &[crate::domain::library::Track],
) -> BackfillYearsScanResult {
    let mut r = BackfillYearsScanResult::default();

    let year_change = |track_id: String, year: i32| TrackChange {
        track_id,
        year: Some(year),
        ..Default::default()
    };

    // Pre-compute normalized keys for all tracks.
    let norm_keys: Vec<(String, String, Option<String>)> = tracks
        .iter()
        .map(|t| {
            let a = normalize_for_matching(&t.artist);
            let ti = normalize_for_matching(&t.title);
            let al = normalize_for_matching(&t.album);
            (a, ti, (!al.is_empty()).then_some(al))
        })
        .collect();

    // Build batch keys for all 4 providers × all tracks.
    let mut enrich_keys: Vec<(&str, &str, &str, &str)> = Vec::with_capacity(tracks.len() * 4);
    for (a, t, al) in &norm_keys {
        let album = al.as_deref().unwrap_or("");
        enrich_keys.push(("discogs", a, t, album));
        enrich_keys.push(("beatport", a, t, ""));
        enrich_keys.push(("musicbrainz", a, t, ""));
        enrich_keys.push(("bandcamp", a, t, ""));
    }

    // Single batch load — replaces up to 8N individual queries.
    let cache_map = state::batch_get_enrichment(store_conn, &enrich_keys).unwrap_or_else(|e| {
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

            let discogs_key = (
                "discogs".to_string(),
                norm_artist.clone(),
                norm_title.clone(),
                album.to_string(),
            );
            let bp_key = (
                "beatport".to_string(),
                norm_artist.clone(),
                norm_title.clone(),
                String::new(),
            );
            let mb_key = (
                "musicbrainz".to_string(),
                norm_artist.clone(),
                norm_title.clone(),
                String::new(),
            );
            let bc_key = (
                "bandcamp".to_string(),
                norm_artist.clone(),
                norm_title.clone(),
                String::new(),
            );

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
            let discogs_key = (
                "discogs".to_string(),
                norm_artist.clone(),
                norm_title.clone(),
                album.to_string(),
            );
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

pub(crate) fn normalize_label(label: &str) -> Option<String> {
    if label.starts_with("Not On Label") {
        return None;
    }
    Some(label.to_string())
}

pub(crate) fn extract_label(entry: Option<&state::EnrichmentCacheEntry>) -> Option<String> {
    parse_response_json(entry)
        .as_ref()
        .and_then(|v| v.get("label"))
        .and_then(|v| v.as_str())
        .filter(|l| !l.is_empty())
        .and_then(normalize_label)
}

pub(crate) fn sort_label_conflicts(conflicts: &mut [serde_json::Value]) {
    conflicts.sort_by(|left, right| {
        let key = |value: &serde_json::Value| {
            (
                normalize_for_matching(value["artist"].as_str().unwrap_or_default()),
                normalize_for_matching(value["title"].as_str().unwrap_or_default()),
                value["track_id"].as_str().unwrap_or_default().to_string(),
            )
        };
        key(left).cmp(&key(right))
    });
}

#[derive(Default)]
pub(crate) struct BackfillLabelsScanResult {
    pub(crate) filled: usize,
    pub(crate) already_labeled: usize,
    pub(crate) conflicts: Vec<serde_json::Value>,
    pub(crate) no_enrichment: usize,
    pub(crate) no_discogs: usize,
    pub(crate) no_musicbrainz: usize,
    pub(crate) no_bandcamp: usize,
    pub(crate) no_beatport: usize,
    pub(crate) to_stage: Vec<TrackChange>,
    /// Tracks that had no Bandcamp cache and got no label from any source.
    pub(crate) uncached_bandcamp: Vec<(String, String, String, String)>, // (norm_artist, norm_title, raw_artist, raw_title)
}

pub(crate) fn scan_labels(
    store_conn: &rusqlite::Connection,
    tracks: &[crate::domain::library::Track],
) -> BackfillLabelsScanResult {
    let mut result = BackfillLabelsScanResult::default();

    // Pre-compute normalized keys for all tracks.
    let norm_keys: Vec<(String, String, Option<String>)> = tracks
        .iter()
        .map(|t| {
            let a = normalize_for_matching(&t.artist);
            let ti = normalize_for_matching(&t.title);
            let al = normalize_for_matching(&t.album);
            (a, ti, (!al.is_empty()).then_some(al))
        })
        .collect();

    // Build batch keys for all 4 providers × all tracks.
    let mut enrich_keys: Vec<(&str, &str, &str, &str)> = Vec::with_capacity(tracks.len() * 4);
    for (a, t, al) in &norm_keys {
        let album = al.as_deref().unwrap_or("");
        enrich_keys.push(("discogs", a, t, album));
        enrich_keys.push(("musicbrainz", a, t, ""));
        enrich_keys.push(("bandcamp", a, t, ""));
        enrich_keys.push(("beatport", a, t, ""));
    }

    // Single batch load — replaces 4N individual queries.
    let cache_map = state::batch_get_enrichment(store_conn, &enrich_keys).unwrap_or_else(|e| {
        tracing::warn!("batch enrichment load failed: {e}");
        std::collections::HashMap::new()
    });

    for (track, (norm_artist, norm_title, norm_album)) in tracks.iter().zip(&norm_keys) {
        let album = norm_album.as_deref().unwrap_or("");

        let discogs_key = (
            "discogs".to_string(),
            norm_artist.clone(),
            norm_title.clone(),
            album.to_string(),
        );
        let mb_key = (
            "musicbrainz".to_string(),
            norm_artist.clone(),
            norm_title.clone(),
            String::new(),
        );
        let bc_key = (
            "bandcamp".to_string(),
            norm_artist.clone(),
            norm_title.clone(),
            String::new(),
        );
        let bp_key = (
            "beatport".to_string(),
            norm_artist.clone(),
            norm_title.clone(),
            String::new(),
        );

        let discogs_entry = cache_map.get(&discogs_key);
        let mb_entry = cache_map.get(&mb_key);
        let bc_entry = cache_map.get(&bc_key);
        let bp_entry = cache_map.get(&bp_key);

        let discogs_label = extract_label(discogs_entry);
        let mb_label = extract_label(mb_entry);
        let bc_label = extract_label(bc_entry);
        let bp_label = extract_label(bp_entry);

        let enrichment_label = discogs_label.or(mb_label).or(bc_label).or(bp_label);

        let Some(enrich_label) = enrichment_label else {
            result.no_enrichment += 1;
            if discogs_entry.is_none() {
                result.no_discogs += 1;
            }
            if mb_entry.is_none() {
                result.no_musicbrainz += 1;
            }
            if bc_entry.is_none() {
                result.no_bandcamp += 1;
                result.uncached_bandcamp.push((
                    norm_artist.clone(),
                    norm_title.clone(),
                    track.artist.clone(),
                    track.title.clone(),
                ));
            }
            if bp_entry.is_none() {
                result.no_beatport += 1;
            }
            continue;
        };

        if track.label.is_empty() {
            result.filled += 1;
            result.to_stage.push(TrackChange {
                track_id: track.id.clone(),
                label: Some(enrich_label.clone()),
                ..Default::default()
            });
        } else if track.label.eq_ignore_ascii_case(&enrich_label) {
            result.already_labeled += 1;
        } else {
            result.conflicts.push(serde_json::json!({
                "track_id": track.id,
                "artist": track.artist,
                "title": track.title,
                "current_label": track.label,
                "enrichment_label": enrich_label,
            }));
        }
    }

    sort_label_conflicts(&mut result.conflicts);
    result
}

/// Known edition/qualifier words — case-insensitive match on parenthetical content.
pub(crate) const QUALIFIER_WORDS: &[&str] = &[
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
pub(crate) fn album_from_folder_path(file_path: &str) -> Option<String> {
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

pub(crate) fn album_from_file_tags(file_path: &str) -> Option<String> {
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

pub(crate) fn is_noise_album(candidate: &str, track_title: &str, artist: &str) -> bool {
    let norm_candidate = normalize_for_matching(candidate);
    let norm_title = normalize_for_matching(track_title);
    let norm_artist = normalize_for_matching(artist);

    norm_candidate == norm_title || norm_candidate == norm_artist
}

#[derive(Default)]
pub(crate) struct BackfillAlbumsScanResult {
    pub(crate) filled: usize,
    pub(crate) already_set: usize,
    pub(crate) no_source: usize,
    pub(crate) skipped_noise: usize,
    pub(crate) no_bandcamp: usize,
    pub(crate) to_stage: Vec<TrackChange>,
    pub(crate) filled_by_source: FilledBySource,
    /// Tracks with no Bandcamp cache and no album from any source.
    pub(crate) uncached_bandcamp: Vec<(String, String, String, String)>,
}

#[derive(Default)]
pub(crate) struct FilledBySource {
    pub(crate) file_tags: usize,
    pub(crate) folder_path: usize,
    pub(crate) bandcamp: usize,
    pub(crate) discogs: usize,
}

pub(crate) fn extract_album(entry: Option<&state::EnrichmentCacheEntry>) -> Option<String> {
    parse_response_json(entry)
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

pub(crate) fn scan_albums(
    store_conn: &rusqlite::Connection,
    tracks: &[crate::domain::library::Track],
) -> BackfillAlbumsScanResult {
    let mut r = BackfillAlbumsScanResult::default();

    // Pre-compute normalized keys for all tracks.
    let norm_keys: Vec<(String, String)> = tracks
        .iter()
        .map(|t| {
            let a = normalize_for_matching(&t.artist);
            let ti = normalize_for_matching(&t.title);
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
    let cache_map = state::batch_get_enrichment(store_conn, &enrich_keys).unwrap_or_else(|e| {
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

#[cfg(test)]
mod workflow_tests {
    use super::*;

    #[test]
    fn boundary_metadata_backfill_stages_through_change_manager() {
        let manager = ChangeManager::new();
        let suggestion = TrackChange {
            track_id: "track-1".to_string(),
            year: Some(2001),
            ..Default::default()
        };
        let (staged, pending) = stage_suggestions(&manager, vec![suggestion], false);
        assert_eq!((staged, pending), (1, 1));
        assert_eq!(manager.pending_ids(), vec!["track-1".to_string()]);
    }
}
