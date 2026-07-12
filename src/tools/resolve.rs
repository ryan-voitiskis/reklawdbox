use rusqlite::Connection;

use rmcp::ErrorData as McpError;

use super::*;
use crate::db;
use crate::genre;
use crate::types::Track;

pub(super) struct ResolveTracksOpts {
    /// Default max_tracks when track_ids are absent and max_tracks param is None.
    /// When track_ids IS present and this is Some, defaults to ids.len().
    /// None = no auto-default (used by cache_coverage).
    pub default_max_tracks: Option<u32>,
    /// Hard cap on effective max. Some(200) for bounded tools, None for unbounded.
    pub max_tracks_cap: Option<u32>,
    /// Post-filter to exclude sampler tracks (used by cache_coverage).
    pub exclude_samplers: bool,
}

pub(super) fn track_has_unknown_genre(track: &Track) -> bool {
    !track.genre.is_empty()
        && !genre::is_known_genre(&track.genre)
        && genre::canonical_genre_from_alias(&track.genre).is_none()
}

pub(super) fn apply_offset_limit(
    tracks: Vec<Track>,
    offset: Option<u32>,
    limit: Option<u32>,
) -> Vec<Track> {
    // Reklawdbox supports 32- and 64-bit targets, where every u32 fits usize.
    let offset = offset.unwrap_or(0) as usize;
    let limit = limit.map_or(usize::MAX, |value| value as usize);
    tracks.into_iter().skip(offset).take(limit).collect()
}

fn required_nullable_offset_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
    <Option<usize> as schemars::JsonSchema>::json_schema(generator)
}

/// Public continuation metadata for bounded work selectors.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, schemars::JsonSchema)]
pub(super) struct BatchPage {
    pub(super) matched_tracks: usize,
    pub(super) start_offset: usize,
    pub(super) examined_tracks: usize,
    pub(super) selected_tracks: usize,
    pub(super) fully_cached_skipped: usize,
    #[schemars(required, schema_with = "required_nullable_offset_schema")]
    pub(super) next_offset: Option<usize>,
    pub(super) has_more: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, schemars::JsonSchema)]
pub(super) struct OffsetPage {
    pub(super) total: usize,
    pub(super) returned: usize,
    pub(super) offset: usize,
    #[schemars(required, schema_with = "required_nullable_offset_schema")]
    pub(super) next_offset: Option<usize>,
    pub(super) has_more: bool,
}

pub(super) fn offset_page_bounds(
    total: usize,
    offset: usize,
    page_size: usize,
) -> (std::ops::Range<usize>, OffsetPage) {
    let start = offset.min(total);
    let returned = if page_size == 0 {
        0
    } else {
        page_size.min(total - start)
    };
    let end = start + returned;
    let has_more = page_size > 0 && offset.saturating_add(returned) < total;
    let next_offset = has_more.then_some(offset.saturating_add(returned));
    (
        start..end,
        OffsetPage {
            total,
            returned,
            offset,
            next_offset,
            has_more,
        },
    )
}

#[derive(Debug)]
pub(super) struct PendingBatchSelection<T> {
    pub(super) selected: Vec<T>,
    pub(super) page: BatchPage,
}

/// Select pending work from an already ordered logical scope.
///
/// The helper never sorts. `is_complete_batch` must return one flag per
/// candidate passed to it, where `true` means the candidate consumes no work
/// slot. The DB-backed wrapper below applies the same contract while streaming
/// rich track rows in bounded chunks.
#[cfg(test)]
pub(super) fn pending_batch_page<T: Clone>(
    ordered: &[T],
    offset: u32,
    max_items: u32,
    mut is_complete_batch: impl FnMut(&[T]) -> Result<Vec<bool>, String>,
) -> Result<PendingBatchSelection<T>, String> {
    let matched_tracks = ordered.len();
    let start_offset = offset as usize;
    if max_items == 0 {
        return Ok(PendingBatchSelection {
            selected: Vec::new(),
            page: BatchPage {
                matched_tracks,
                start_offset,
                examined_tracks: 0,
                selected_tracks: 0,
                fully_cached_skipped: 0,
                next_offset: None,
                has_more: false,
            },
        });
    }

    let start = start_offset.min(matched_tracks);
    let candidates = &ordered[start..];
    let complete = is_complete_batch(candidates)?;
    if complete.len() != candidates.len() {
        return Err(format!(
            "completion predicate returned {} flags for {} candidates",
            complete.len(),
            candidates.len()
        ));
    }

    let max_items = max_items as usize;
    let mut selected = Vec::with_capacity(max_items.min(candidates.len()));
    let mut examined_tracks = 0usize;
    let mut fully_cached_skipped = 0usize;
    let mut after_last_examined = None;

    for (local_index, (candidate, is_complete)) in
        candidates.iter().zip(complete.into_iter()).enumerate()
    {
        examined_tracks += 1;
        after_last_examined = Some(start + local_index + 1);
        if is_complete {
            fully_cached_skipped += 1;
        } else {
            selected.push(candidate.clone());
            if selected.len() == max_items {
                break;
            }
        }
    }

    let has_more = after_last_examined.is_some_and(|next| next < matched_tracks);
    let next_offset = after_last_examined.filter(|_| has_more);
    let selected_tracks = selected.len();
    Ok(PendingBatchSelection {
        selected,
        page: BatchPage {
            matched_tracks,
            start_offset,
            examined_tracks,
            selected_tracks,
            fully_cached_skipped,
            next_offset,
            has_more,
        },
    })
}

const BATCH_SELECTOR_CHUNK: usize = 200;

fn filter_logical_tracks(
    mut tracks: Vec<Track>,
    has_unknown_genre: bool,
    exclude_samplers: bool,
) -> Vec<Track> {
    if exclude_samplers {
        tracks.retain(|track| !track.file_path.contains(db::SAMPLER_PATH_FRAGMENT));
    }
    if has_unknown_genre {
        tracks.retain(track_has_unknown_genre);
    }
    tracks
}

/// Visit a logical selector in stable order without retaining its full rich
/// track payload in memory.
fn visit_ordered_track_chunks(
    conn: &Connection,
    track_ids: Option<&[String]>,
    playlist_id: Option<&str>,
    mut filters: SearchFilterParams,
    exclude_samplers: bool,
    mut visit: impl FnMut(Vec<Track>) -> Result<(), McpError>,
) -> Result<(), McpError> {
    if filters.has_unknown_genre == Some(true) && filters.has_genre.is_none() {
        filters.has_genre = Some(true);
    }
    let has_unknown_genre = filters.has_unknown_genre == Some(true);

    if let Some(ids) = track_ids {
        let mut seen = std::collections::HashSet::new();
        let mut unique_chunk = Vec::with_capacity(BATCH_SELECTOR_CHUNK);
        for id in ids {
            if !seen.insert(id.as_str()) {
                continue;
            }
            unique_chunk.push(id.clone());
            if unique_chunk.len() == BATCH_SELECTOR_CHUNK {
                let tracks = db::get_tracks_by_ids(conn, &unique_chunk).map_err(db_error)?;
                visit(filter_logical_tracks(
                    tracks,
                    has_unknown_genre,
                    exclude_samplers,
                ))?;
                unique_chunk.clear();
            }
        }
        if !unique_chunk.is_empty() {
            let tracks = db::get_tracks_by_ids(conn, &unique_chunk).map_err(db_error)?;
            visit(filter_logical_tracks(
                tracks,
                has_unknown_genre,
                exclude_samplers,
            ))?;
        }
        return Ok(());
    }

    if let Some(playlist_id) = playlist_id {
        let mut raw_offset = 0u32;
        loop {
            let tracks = db::get_playlist_tracks_unbounded_page(
                conn,
                playlist_id,
                Some(BATCH_SELECTOR_CHUNK as u32),
                Some(raw_offset),
            )
            .map_err(db_error)?;
            let raw_count = tracks.len();
            if raw_count == 0 {
                break;
            }
            visit(filter_logical_tracks(
                tracks,
                has_unknown_genre,
                exclude_samplers,
            ))?;
            if raw_count < BATCH_SELECTOR_CHUNK {
                break;
            }
            raw_offset = raw_offset
                .checked_add(BATCH_SELECTOR_CHUNK as u32)
                .ok_or_else(|| mcp_internal_error("playlist selector offset exceeds u32"))?;
        }
        return Ok(());
    }

    let mut search = filters
        .into_search_params(exclude_samplers, Some(BATCH_SELECTOR_CHUNK as u32), Some(0))
        .map_err(|error| McpError::invalid_params(error, None))?;
    let mut raw_offset = 0u32;
    loop {
        search.offset = Some(raw_offset);
        let tracks = db::search_tracks_unbounded(conn, &search).map_err(db_error)?;
        let raw_count = tracks.len();
        if raw_count == 0 {
            break;
        }
        visit(filter_logical_tracks(
            tracks,
            has_unknown_genre,
            exclude_samplers,
        ))?;
        if raw_count < BATCH_SELECTOR_CHUNK {
            break;
        }
        raw_offset = raw_offset
            .checked_add(BATCH_SELECTOR_CHUNK as u32)
            .ok_or_else(|| mcp_internal_error("search selector offset exceeds u32"))?;
    }
    Ok(())
}

/// Resolve and select bounded pending work while keeping logical traversal
/// memory-bounded. Only selected rich track rows are retained after each
/// 200-row selector chunk.
#[allow(clippy::too_many_arguments)]
pub(super) fn resolve_pending_tracks(
    conn: &Connection,
    track_ids: Option<&[String]>,
    playlist_id: Option<&str>,
    filters: SearchFilterParams,
    max_tracks_param: Option<u32>,
    offset: Option<u32>,
    default_max_tracks: u32,
    max_tracks_cap: u32,
    exclude_samplers: bool,
    mut is_complete_batch: impl FnMut(&[Track]) -> Result<Vec<bool>, McpError>,
) -> Result<PendingBatchSelection<Track>, McpError> {
    let max_tracks = max_tracks_param
        .unwrap_or_else(|| {
            track_ids.map_or(default_max_tracks, |ids| {
                u32::try_from(ids.len()).unwrap_or(u32::MAX)
            })
        })
        .min(max_tracks_cap);
    let start_offset = offset.unwrap_or(0) as usize;

    let mut matched_tracks = 0usize;
    let mut selected = Vec::with_capacity(max_tracks as usize);
    let mut examined_tracks = 0usize;
    let mut fully_cached_skipped = 0usize;
    let mut after_last_examined = None;

    visit_ordered_track_chunks(
        conn,
        track_ids,
        playlist_id,
        filters,
        exclude_samplers,
        |chunk| {
            let chunk_start = matched_tracks;
            matched_tracks = matched_tracks.saturating_add(chunk.len());
            if max_tracks == 0 || selected.len() == max_tracks as usize {
                return Ok(());
            }

            let local_start = start_offset.saturating_sub(chunk_start).min(chunk.len());
            let candidates = &chunk[local_start..];
            if candidates.is_empty() {
                return Ok(());
            }
            let complete = is_complete_batch(candidates)?;
            if complete.len() != candidates.len() {
                return Err(mcp_internal_error(format!(
                    "completion predicate returned {} flags for {} candidates",
                    complete.len(),
                    candidates.len()
                )));
            }

            for (local_index, (track, is_complete)) in chunk
                .into_iter()
                .skip(local_start)
                .zip(complete.into_iter())
                .enumerate()
            {
                examined_tracks += 1;
                after_last_examined = Some(chunk_start + local_start + local_index + 1);
                if is_complete {
                    fully_cached_skipped += 1;
                } else {
                    selected.push(track);
                    if selected.len() == max_tracks as usize {
                        break;
                    }
                }
            }
            Ok(())
        },
    )?;

    let (next_offset, has_more) = if max_tracks == 0 {
        (None, false)
    } else {
        let has_more = after_last_examined.is_some_and(|next| next < matched_tracks);
        (after_last_examined.filter(|_| has_more), has_more)
    };
    let selected_tracks = selected.len();
    Ok(PendingBatchSelection {
        selected,
        page: BatchPage {
            matched_tracks,
            start_offset,
            examined_tracks,
            selected_tracks,
            fully_cached_skipped,
            next_offset,
            has_more,
        },
    })
}

/// Resolve tracks using priority: track_ids > playlist_id > search filters.
///
/// Shared by `enrich_tracks`, `analyze_audio_batch`, `resolve_tracks_data`, and `cache_coverage`.
pub(super) fn resolve_tracks(
    conn: &Connection,
    track_ids: Option<&[String]>,
    playlist_id: Option<&str>,
    mut filters: SearchFilterParams,
    max_tracks_param: Option<u32>,
    offset: Option<u32>,
    opts: &ResolveTracksOpts,
) -> Result<Vec<crate::types::Track>, McpError> {
    let effective_max: Option<u32> = match opts.default_max_tracks {
        Some(default_when_no_ids) => {
            let default = track_ids.map_or(default_when_no_ids, |ids| {
                u32::try_from(ids.len()).unwrap_or(u32::MAX)
            });
            let mut max = max_tracks_param.unwrap_or(default);
            if let Some(max_tracks_cap) = opts.max_tracks_cap {
                max = max.min(max_tracks_cap);
            }
            Some(max)
        }
        None => max_tracks_param.map(|m| {
            if let Some(max_tracks_cap) = opts.max_tracks_cap {
                m.min(max_tracks_cap)
            } else {
                m
            }
        }),
    };

    if filters.has_unknown_genre == Some(true) && filters.has_genre.is_none() {
        filters.has_genre = Some(true);
    }

    let has_unknown_genre = filters.has_unknown_genre;
    let bounded = opts.max_tracks_cap.is_some();

    // Selector priority is IDs > playlist > search. Pagination is applied in
    // SQL only when every logical filter is also in SQL; otherwise the full
    // ordered candidate set is post-filtered and paginated locally.
    let (tracks, pagination_applied_in_db) = if let Some(ids) = track_ids {
        (db::get_tracks_by_ids(conn, ids).map_err(db_error)?, false)
    } else if let Some(pid) = playlist_id {
        let playlist_requires_post_filter =
            has_unknown_genre == Some(true) || opts.exclude_samplers;
        if playlist_requires_post_filter {
            (
                db::get_playlist_tracks_unbounded(conn, pid, None).map_err(db_error)?,
                false,
            )
        } else if bounded {
            (
                db::get_playlist_tracks_page(conn, pid, effective_max, offset).map_err(db_error)?,
                true,
            )
        } else {
            (
                db::get_playlist_tracks_unbounded_page(conn, pid, effective_max, offset)
                    .map_err(db_error)?,
                true,
            )
        }
    } else {
        if has_unknown_genre == Some(true) {
            let search = filters
                .into_search_params(true, None, None)
                .map_err(|e| McpError::invalid_params(e, None))?;
            (
                db::search_tracks_unbounded(conn, &search).map_err(db_error)?,
                false,
            )
        } else {
            let search = filters
                .into_search_params(true, effective_max, offset)
                .map_err(|e| McpError::invalid_params(e, None))?;
            if bounded {
                (db::search_tracks(conn, &search).map_err(db_error)?, true)
            } else {
                (
                    db::search_tracks_unbounded(conn, &search).map_err(db_error)?,
                    true,
                )
            }
        }
    };

    let mut tracks: Vec<_> = if opts.exclude_samplers {
        tracks
            .into_iter()
            .filter(|t| !t.file_path.contains(db::SAMPLER_PATH_FRAGMENT))
            .collect()
    } else {
        tracks
    };

    if has_unknown_genre == Some(true) {
        tracks.retain(track_has_unknown_genre);
    }

    if pagination_applied_in_db {
        Ok(tracks)
    } else {
        Ok(apply_offset_limit(tracks, offset, effective_max))
    }
}

pub(super) fn describe_resolve_scope(params: &ResolveTracksDataParams) -> String {
    if let Some(track_ids) = &params.track_ids {
        if let Some(max_tracks) = params.max_tracks {
            return format!(
                "track_ids ({}) [max_tracks = {max_tracks}]",
                track_ids.len()
            );
        }
        return format!("track_ids ({})", track_ids.len());
    }

    if let Some(playlist_id) = &params.playlist_id {
        if let Some(max_tracks) = params.max_tracks {
            return format!("playlist_id = \"{playlist_id}\", max_tracks = {max_tracks}");
        }
        return format!("playlist_id = \"{playlist_id}\"");
    }

    let mut filters: Vec<String> = Vec::new();
    if let Some(query) = &params.filters.query {
        filters.push(format!("query ~= \"{query}\""));
    }
    if let Some(artist) = &params.filters.artist {
        filters.push(format!("artist ~= \"{artist}\""));
    }
    if let Some(genre) = &params.filters.genre {
        filters.push(format!("genre ~= \"{genre}\""));
    }
    if let Some(has_genre) = params.filters.has_genre {
        filters.push(format!("has_genre = {has_genre}"));
    }
    if let Some(has_label) = params.filters.has_label {
        filters.push(format!("has_label = {has_label}"));
    }
    if let Some(has_unknown_genre) = params.filters.has_unknown_genre {
        filters.push(format!("has_unknown_genre = {has_unknown_genre}"));
    }
    if let Some(year_zero) = params.filters.year_zero {
        filters.push(format!("year_zero = {year_zero}"));
    }
    if let Some(bpm_min) = params.filters.bpm_min {
        filters.push(format!("bpm_min = {bpm_min}"));
    }
    if let Some(bpm_max) = params.filters.bpm_max {
        filters.push(format!("bpm_max = {bpm_max}"));
    }
    if let Some(key) = &params.filters.key {
        filters.push(format!("key = \"{key}\""));
    }
    if let Some(rating_min) = params.filters.rating_min {
        filters.push(format!("rating_min = {rating_min}"));
    }
    if let Some(label) = &params.filters.label {
        filters.push(format!("label ~= \"{label}\""));
    }
    if let Some(path) = &params.filters.path {
        filters.push(format!("path ~= \"{path}\""));
    }
    if let Some(prefix) = &params.filters.path_prefix {
        filters.push(format!("path_prefix = \"{prefix}\""));
    }
    if let Some(added_after) = &params.filters.added_after {
        filters.push(format!("added_after = \"{added_after}\""));
    }
    if let Some(added_before) = &params.filters.added_before {
        filters.push(format!("added_before = \"{added_before}\""));
    }
    if let Some(max_tracks) = params.max_tracks {
        filters.push(format!("max_tracks = {max_tracks}"));
    }

    if filters.is_empty() {
        "all tracks".to_string()
    } else {
        filters.join(", ")
    }
}

pub(super) fn to_percent(count: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        ((count as f64 / total as f64) * 1000.0).round() / 10.0
    }
}

#[cfg(test)]
mod pending_page_characterization_tests {
    use super::*;

    fn track(id: &str) -> Track {
        Track {
            id: id.to_string(),
            title: id.to_string(),
            artist: String::new(),
            album: String::new(),
            genre: String::new(),
            bpm: 0.0,
            key: String::new(),
            rating: 0,
            comments: String::new(),
            color: String::new(),
            color_code: 0,
            label: String::new(),
            remixer: String::new(),
            year: 0,
            length: 0,
            file_path: String::new(),
            play_count: 0,
            bit_rate: 0,
            sample_rate: 0,
            file_kind: crate::types::FileKind::Unknown(0),
            date_added: String::new(),
            position: None,
            played_at: None,
        }
    }

    fn select(
        ids: &[&str],
        complete_ids: &[&str],
        offset: u32,
        limit: u32,
    ) -> PendingBatchSelection<Track> {
        let tracks: Vec<_> = ids.iter().map(|id| track(id)).collect();
        pending_batch_page(&tracks, offset, limit, |candidates| {
            Ok(candidates
                .iter()
                .map(|track| complete_ids.contains(&track.id.as_str()))
                .collect())
        })
        .expect("pending page should resolve")
    }

    fn selected_ids(selection: &PendingBatchSelection<Track>) -> Vec<&str> {
        selection
            .selected
            .iter()
            .map(|track| track.id.as_str())
            .collect()
    }

    #[test]
    fn pending_batch_page_reaches_work_after_a_complete_prefix() {
        let selection = select(
            &["cached-1", "cached-2", "cached-3", "pending-1"],
            &["cached-1", "cached-2", "cached-3"],
            0,
            2,
        );

        assert_eq!(selected_ids(&selection), ["pending-1"]);
        assert_eq!(
            selection.page,
            BatchPage {
                matched_tracks: 4,
                start_offset: 0,
                examined_tracks: 4,
                selected_tracks: 1,
                fully_cached_skipped: 3,
                next_offset: None,
                has_more: false,
            }
        );
    }

    #[test]
    fn pending_batch_page_interleaves_complete_and_pending_without_sorting() {
        let selection = select(&["p-1", "c-1", "p-2", "c-2", "p-3"], &["c-1", "c-2"], 0, 2);

        assert_eq!(selected_ids(&selection), ["p-1", "p-2"]);
        assert_eq!(
            selection.page,
            BatchPage {
                matched_tracks: 5,
                start_offset: 0,
                examined_tracks: 3,
                selected_tracks: 2,
                fully_cached_skipped: 1,
                next_offset: Some(3),
                has_more: true,
            }
        );
    }

    #[test]
    fn pending_batch_page_offsets_before_inside_and_beyond_scope() {
        let inside = select(&["c-1", "p-1", "p-2"], &["c-1"], 1, 1);
        assert_eq!(selected_ids(&inside), ["p-1"]);
        assert_eq!(
            inside.page,
            BatchPage {
                matched_tracks: 3,
                start_offset: 1,
                examined_tracks: 1,
                selected_tracks: 1,
                fully_cached_skipped: 0,
                next_offset: Some(2),
                has_more: true,
            }
        );

        let beyond = select(&["p-1", "p-2"], &[], u32::MAX, 200);
        assert!(beyond.selected.is_empty());
        assert_eq!(
            beyond.page,
            BatchPage {
                matched_tracks: 2,
                start_offset: u32::MAX as usize,
                examined_tracks: 0,
                selected_tracks: 0,
                fully_cached_skipped: 0,
                next_offset: None,
                has_more: false,
            }
        );
    }

    #[test]
    fn pending_batch_page_zero_limit_is_a_terminal_no_op() {
        let selection = select(&["p-1", "p-2"], &[], 1, 0);
        assert!(selection.selected.is_empty());
        assert_eq!(
            selection.page,
            BatchPage {
                matched_tracks: 2,
                start_offset: 1,
                examined_tracks: 0,
                selected_tracks: 0,
                fully_cached_skipped: 0,
                next_offset: None,
                has_more: false,
            }
        );
    }

    #[test]
    fn pending_batch_page_failure_can_be_bypassed_with_next_offset() {
        // Pending status is intentionally unchanged after a failed attempt.
        let first = select(&["failed", "later"], &[], 0, 1);
        assert_eq!(selected_ids(&first), ["failed"]);
        assert_eq!(first.page.next_offset, Some(1));
        assert!(first.page.has_more);

        let continuation = select(&["failed", "later"], &[], 1, 1);
        assert_eq!(selected_ids(&continuation), ["later"]);
        assert_eq!(
            continuation.page,
            BatchPage {
                matched_tracks: 2,
                start_offset: 1,
                examined_tracks: 1,
                selected_tracks: 1,
                fully_cached_skipped: 0,
                next_offset: None,
                has_more: false,
            }
        );
    }
}
