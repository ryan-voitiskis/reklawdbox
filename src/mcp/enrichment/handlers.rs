use rmcp::ErrorData as McpError;
use rmcp::model::CallToolResult;

use crate::adapters::providers::musicbrainz;
use crate::adapters::rekordbox as db;
use crate::adapters::state as store;
use crate::application::analysis::batch::CacheWriteRequest;
#[cfg(test)]
use crate::application::enrichment::hydrate::acknowledge_mcp_enrichment_cache_write;
use crate::application::enrichment::hydrate::{
    EnrichmentCachePolicy, EnrichmentCacheWrite, EnrichmentCacheWriterReport,
    EnrichmentWorkerConfig, HydrationFailure, HydrationFailureKind, HydrationTrackIdentity,
    enrichment_completion_flags, run_enrichment_cache_writer, run_enrichment_workers,
};
use crate::application::enrichment::lookup::{
    self as enrichment_lookup, LookupIdentity, LookupPolicy, LookupProvider, PersistLookupError,
};
use crate::application::enrichment::model::CacheLookupOutcome;
use crate::mcp::{
    BatchPage, BatchProgress, EnrichTracksParams, LookupBandcampParams, LookupDiscogsParams,
    LookupMusicBrainzParams, ReklawdboxServer, auth_remediation_message, cache_error, db_error,
    lookup_discogs_remote, mcp_internal_error, ok_json, ok_structured_json, resolve_pending_tracks,
};

fn cached_lookup(
    server: &ReklawdboxServer,
    provider: LookupProvider,
    identity: &LookupIdentity,
    policy: LookupPolicy,
) -> Result<Option<enrichment_lookup::LookupResult>, McpError> {
    let store_conn = server.cache_store_conn()?;
    match enrichment_lookup::read_lookup_cache(&store_conn, provider, identity, policy)
        .map_err(cache_error)?
    {
        CacheLookupOutcome::Hit(result) => Ok(Some(result)),
        CacheLookupOutcome::Miss => Ok(None),
    }
}

fn persist_lookup_result<T>(
    server: &ReklawdboxServer,
    persist: impl FnOnce(&rusqlite::Connection) -> Result<T, PersistLookupError>,
) -> Result<T, McpError> {
    let store_conn = server.cache_store_conn()?;
    persist(&store_conn).map_err(|error| match error {
        PersistLookupError::Cache(error) => cache_error(error),
        PersistLookupError::Serialize(error) => mcp_internal_error(error.to_string()),
    })
}

pub(in crate::mcp) async fn lookup_bandcamp_remote(
    server: &ReklawdboxServer,
    artist: &str,
    title: &str,
) -> Result<Option<crate::adapters::providers::bandcamp::BandcampResult>, String> {
    #[cfg(test)]
    if let Some(result) = super::core::take_test_bandcamp_lookup_override(artist, title) {
        return result;
    }
    let identity = LookupIdentity::new(artist.to_string(), title.to_string(), None);
    enrichment_lookup::dispatch_bandcamp(&server.context.enrichment.http, &identity, None)
        .await
        .map_err(|error| error.to_string())
}

/// Resolve track identity from either track_id (DB lookup) or explicit artist/title.
fn resolve_lookup_identity(
    server: &ReklawdboxServer,
    track_id: Option<&str>,
    artist: Option<String>,
    title: Option<String>,
    album: Option<String>,
) -> Result<(String, String, Option<String>), McpError> {
    if let Some(track_id) = track_id {
        let conn = server.rekordbox_conn()?;
        let track = db::get_track(&conn, track_id)
            .map_err(db_error)?
            .ok_or_else(|| {
                McpError::invalid_params(format!("Track '{track_id}' not found"), None)
            })?;
        let album = album.or_else(|| (!track.album.is_empty()).then(|| track.album.clone()));
        Ok((
            artist.unwrap_or(track.artist),
            title.unwrap_or(track.title),
            album,
        ))
    } else {
        let artist = artist.ok_or_else(|| {
            McpError::invalid_params("artist is required when track_id is not provided", None)
        })?;
        let title = title.ok_or_else(|| {
            McpError::invalid_params("title is required when track_id is not provided", None)
        })?;
        Ok((artist, title, album))
    }
}

pub(in crate::mcp) async fn handle_lookup_discogs(
    server: &ReklawdboxServer,
    params: LookupDiscogsParams,
) -> Result<CallToolResult, McpError> {
    let force_refresh = params.force_refresh.unwrap_or(false);

    let (artist, title, album) = resolve_lookup_identity(
        server,
        params.track_id.as_deref(),
        params.artist,
        params.title,
        params.album,
    )?;

    let identity = LookupIdentity::new(artist, title, album);
    let policy = LookupPolicy {
        force_refresh,
        cache_read_enabled: true,
    };
    if let Some(cached) = cached_lookup(server, LookupProvider::Discogs, &identity, policy)? {
        return ok_json(&cached.into_output());
    }

    let result = lookup_discogs_remote(
        server,
        &identity.artist,
        &identity.title,
        identity.album.as_deref(),
    )
    .await
    .map_err(|error| match error.auth_remediation() {
        Some(remediation) => mcp_internal_error(auth_remediation_message(remediation)),
        None => mcp_internal_error(format!("Discogs error: {error}")),
    })?;
    let result = persist_lookup_result(server, |conn| {
        enrichment_lookup::persist_discogs_result(conn, &identity, result)
    })?;

    ok_json(&result.into_output())
}

pub(in crate::mcp) async fn handle_lookup_musicbrainz(
    server: &ReklawdboxServer,
    params: LookupMusicBrainzParams,
) -> Result<CallToolResult, McpError> {
    let force_refresh = params.force_refresh.unwrap_or(false);

    let (artist, title, _) = resolve_lookup_identity(
        server,
        params.track_id.as_deref(),
        params.artist,
        params.title,
        None,
    )?;

    let identity = LookupIdentity::new(artist, title, None);
    let policy = LookupPolicy {
        force_refresh,
        cache_read_enabled: true,
    };
    if let Some(cached) = cached_lookup(server, LookupProvider::MusicBrainz, &identity, policy)? {
        return ok_json(&cached.into_output());
    }

    let result =
        enrichment_lookup::dispatch_musicbrainz(&server.context.enrichment.http, &identity)
            .await
            .map_err(|error| mcp_internal_error(format!("MusicBrainz error: {error}")))?;
    let result = persist_lookup_result(server, |conn| {
        enrichment_lookup::persist_musicbrainz_result(conn, &identity, result)
    })?;

    ok_json(&result.into_output())
}

pub(in crate::mcp) async fn handle_lookup_bandcamp(
    server: &ReklawdboxServer,
    params: LookupBandcampParams,
) -> Result<CallToolResult, McpError> {
    let force_refresh = params.force_refresh.unwrap_or(false);
    let url = params.url;

    let (artist, title, _) = resolve_lookup_identity(
        server,
        params.track_id.as_deref(),
        params.artist,
        params.title,
        None,
    )?;

    let identity = LookupIdentity::new(artist, title, None);
    let policy = LookupPolicy {
        force_refresh,
        cache_read_enabled: url.is_none(),
    };
    if let Some(cached) = cached_lookup(server, LookupProvider::Bandcamp, &identity, policy)? {
        return ok_json(&cached.into_output());
    }

    let result = enrichment_lookup::dispatch_bandcamp(
        &server.context.enrichment.http,
        &identity,
        url.as_deref(),
    )
    .await
    .map_err(|error| mcp_internal_error(format!("Bandcamp error: {error}")))?;
    let result = persist_lookup_result(server, |conn| {
        enrichment_lookup::persist_bandcamp_result(conn, &identity, result)
    })?;

    ok_json(&result.into_output())
}

pub(in crate::mcp) async fn lookup_musicbrainz_remote(
    server: &ReklawdboxServer,
    artist: &str,
    title: &str,
) -> Result<Option<musicbrainz::MusicBrainzResult>, String> {
    #[cfg(test)]
    if let Some(result) = super::core::take_test_musicbrainz_lookup_override(artist, title) {
        return result;
    }
    let identity = LookupIdentity::new(artist.to_string(), title.to_string(), None);
    enrichment_lookup::dispatch_musicbrainz(&server.context.enrichment.http, &identity)
        .await
        .map_err(|error| error.to_string())
}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub(in crate::mcp) struct EnrichCacheWriteSummary {
    attempted: usize,
    succeeded: usize,
    failed: usize,
}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub(in crate::mcp) struct EnrichTracksSummary {
    tracks_total: usize,
    total: usize,
    enriched: usize,
    cached: usize,
    skipped: usize,
    failed: usize,
    concurrency: usize,
    cache_writes: EnrichCacheWriteSummary,
}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub(in crate::mcp) struct EnrichTracksOutput {
    summary: EnrichTracksSummary,
    failures: Vec<serde_json::Value>,
    page: BatchPage,
}

fn enrich_page_summary_counts(
    page: &BatchPage,
    provider_count: usize,
    selected_cached: usize,
) -> (usize, usize, usize) {
    let tracks_total = page.examined_tracks;
    let total = tracks_total.saturating_mul(provider_count);
    let fully_cached = page.fully_cached_skipped.saturating_mul(provider_count);
    let cached = selected_cached.saturating_add(fully_cached);
    (tracks_total, total, cached)
}

fn enrichment_join_failures(
    track_id: &str,
    artist: &str,
    title: &str,
    providers: &[crate::application::enrichment::model::EnrichmentProvider],
    stage: &str,
    error: &str,
) -> Vec<serde_json::Value> {
    providers
        .iter()
        .map(|provider| {
            serde_json::json!({
                "track_id": track_id,
                "artist": artist,
                "title": title,
                "provider": provider.as_str(),
                "stage": stage,
                "error": error,
            })
        })
        .collect()
}

fn hydration_identity(track: &crate::domain::library::Track) -> HydrationTrackIdentity {
    HydrationTrackIdentity::new(
        track.id.clone(),
        track.artist.clone(),
        track.title.clone(),
        track.album.clone(),
    )
}

fn hydration_failure_json(failure: HydrationFailure) -> serde_json::Value {
    let stage = failure.kind.stage();
    let error = match failure.kind {
        HydrationFailureKind::AuthBatchFailed => "Discogs auth failed (batch-wide)".to_string(),
        HydrationFailureKind::DiscogsAuth(remediation) => auth_remediation_message(&remediation),
        HydrationFailureKind::Lookup(error)
        | HydrationFailureKind::Serialize(error)
        | HydrationFailureKind::CacheWrite(error) => error,
        HydrationFailureKind::SemaphoreClosed => {
            format!("{} semaphore closed", failure.provider)
        }
    };
    serde_json::json!({
        "track_id": failure.identity.track_id,
        "artist": failure.identity.artist,
        "title": failure.identity.title,
        "provider": failure.provider.as_str(),
        "stage": stage,
        "error": error,
    })
}

pub(in crate::mcp) async fn handle_enrich_tracks(
    server: &ReklawdboxServer,
    params: EnrichTracksParams,
) -> Result<CallToolResult, McpError> {
    let skip_cached = params.skip_cached.unwrap_or(true);
    let force_refresh = params.force_refresh.unwrap_or(false);
    let providers = params.providers.unwrap_or_else(|| {
        vec![crate::application::enrichment::model::EnrichmentProvider::Discogs]
    });
    let store_path = server.cache_store_path();

    // Initialize/migrate the store without holding its MutexGuard alongside
    // the Rekordbox guard. Pending selection uses a dedicated read-only
    // connection, avoiding cross-tool lock-order deadlocks.
    {
        let _store_guard = server.cache_store_conn()?;
    }

    let selection = {
        let store_conn = if skip_cached && !force_refresh {
            Some(store::open_read_only(&store_path).map_err(cache_error)?)
        } else {
            None
        };
        let conn = server.rekordbox_conn()?;
        if let Some(store_conn) = store_conn.as_ref() {
            resolve_pending_tracks(
                &conn,
                params.track_ids.as_deref(),
                params.playlist_id.as_deref(),
                params.filters,
                params.max_tracks,
                params.offset,
                50,
                200,
                false,
                |tracks| {
                    let identities: Vec<_> = tracks.iter().map(hydration_identity).collect();
                    enrichment_completion_flags(store_conn, &identities, &providers)
                        .map_err(cache_error)
                },
            )?
        } else {
            resolve_pending_tracks(
                &conn,
                params.track_ids.as_deref(),
                params.playlist_id.as_deref(),
                params.filters,
                params.max_tracks,
                params.offset,
                50,
                200,
                false,
                |tracks| Ok(vec![false; tracks.len()]),
            )?
        }
    };
    let tracks = selection.selected;
    let page = selection.page;

    let concurrency = params.concurrency.map_or(4, |n| n.clamp(1, 8)) as usize;

    let (cache_tx, cache_rx) =
        tokio::sync::mpsc::channel::<CacheWriteRequest<EnrichmentCacheWrite>>(concurrency * 4);
    let writer_store_path = store_path.clone();
    let writer_handle = tokio::task::spawn_blocking(move || {
        run_enrichment_cache_writer(&writer_store_path, cache_rx)
    });

    let (auth_fail_tx, auth_fail_rx) = tokio::sync::watch::channel(false);
    let auth_fail_tx = std::sync::Arc::new(auth_fail_tx);
    let auth_fail_rx = std::sync::Arc::new(auth_fail_rx);

    let worker_report = run_enrichment_workers(
        tracks.iter().map(hydration_identity).collect(),
        EnrichmentWorkerConfig {
            providers: providers.clone(),
            policy: EnrichmentCachePolicy {
                skip_cached,
                force_refresh,
            },
            store_path: store_path.clone(),
            concurrency,
        },
        server.context.enrichment.http.clone(),
        cache_tx.clone(),
        auth_fail_rx,
        auth_fail_tx,
        {
            let server = server.clone();
            move |identity: HydrationTrackIdentity| {
                let server = server.clone();
                async move {
                    let album = (!identity.album.is_empty()).then_some(identity.album.as_str());
                    lookup_discogs_remote(&server, &identity.artist, &identity.title, album).await
                }
            }
        },
    )
    .await;

    let mut progress = BatchProgress::new();
    for (_, track_result) in worker_report.completed {
        progress.processed += track_result.enriched;
        progress.cached += track_result.cached;
        progress.skipped += track_result.no_match;
        progress.failures.extend(
            track_result
                .failures
                .into_iter()
                .map(hydration_failure_json),
        );
    }
    for failure in worker_report.join_failures {
        progress.failures.extend(enrichment_join_failures(
            &failure.identity.track_id,
            &failure.identity.artist,
            &failure.identity.title,
            &providers,
            "task_join",
            &format!("Task panicked: {}", failure.error),
        ));
    }

    drop(cache_tx);
    let cache_write_report = match writer_handle.await {
        Ok(report) => {
            debug_assert_eq!(report.attempted, report.succeeded + report.failed);
            report
        }
        Err(e) => {
            for track in &tracks {
                progress.failures.extend(enrichment_join_failures(
                    &track.id,
                    &track.artist,
                    &track.title,
                    &providers,
                    "cache_writer_join",
                    &format!("Cache writer task failed: {e}"),
                ));
            }
            EnrichmentCacheWriterReport::default()
        }
    };

    let (tracks_total, total, cached) =
        enrich_page_summary_counts(&page, providers.len(), progress.cached);
    let result = EnrichTracksOutput {
        summary: EnrichTracksSummary {
            tracks_total,
            total,
            enriched: progress.processed,
            cached,
            skipped: progress.skipped,
            failed: progress.failures.len(),
            concurrency,
            cache_writes: EnrichCacheWriteSummary {
                attempted: cache_write_report.attempted,
                succeeded: cache_write_report.succeeded,
                failed: cache_write_report.failed,
            },
        },
        failures: progress.failures,
        page,
    };
    ok_structured_json(result)
}

#[cfg(test)]
mod pending_page_tests {
    use super::*;
    use crate::mcp::enrichment::pending_batch_page;

    fn track(id: &str, album: &str) -> crate::domain::library::Track {
        crate::domain::library::Track {
            id: id.to_string(),
            title: "Shared Title".to_string(),
            artist: "Shared Artist".to_string(),
            album: album.to_string(),
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
            file_kind: crate::domain::library::FileKind::Unknown(0),
            date_added: String::new(),
            position: None,
            played_at: None,
        }
    }

    fn store() -> (tempfile::TempDir, rusqlite::Connection) {
        let dir = tempfile::tempdir().expect("temporary store directory should create");
        let path = dir.path().join("store.sqlite3");
        let conn = crate::adapters::state::open(path.to_str().expect("store path should be UTF-8"))
            .expect("temporary store should open");
        (dir, conn)
    }

    fn cache(conn: &rusqlite::Connection, provider: &str, album: Option<&str>, quality: &str) {
        crate::adapters::state::set_enrichment(
            conn,
            provider,
            &crate::domain::metadata::normalize_for_matching("Shared Artist"),
            &crate::domain::metadata::normalize_for_matching("Shared Title"),
            album,
            Some(quality),
            None,
        )
        .expect("cache fixture should write");
    }

    fn completion(
        conn: &rusqlite::Connection,
        tracks: &[crate::domain::library::Track],
        providers: &[crate::application::enrichment::model::EnrichmentProvider],
    ) -> Result<Vec<bool>, rusqlite::Error> {
        let identities: Vec<_> = tracks.iter().map(hydration_identity).collect();
        enrichment_completion_flags(conn, &identities, providers)
    }

    #[test]
    fn enrich_tracks_pending_page_uses_exact_album_and_no_match_is_complete() {
        let (_dir, conn) = store();
        let tracks = vec![
            track("release-a", "Release A"),
            track("release-b", "Release B"),
        ];
        let album_a = crate::domain::metadata::normalize_for_matching("Release A");
        cache(&conn, "discogs", Some(&album_a), "none");
        cache(&conn, "bandcamp", None, "exact");

        let complete = completion(
            &conn,
            &tracks,
            &[
                crate::application::enrichment::model::EnrichmentProvider::Discogs,
                crate::application::enrichment::model::EnrichmentProvider::Bandcamp,
            ],
        )
        .expect("completion lookup should succeed");
        assert_eq!(complete, [true, false]);

        let selection = pending_batch_page(&tracks, 0, 1, |_| Ok(complete.clone()))
            .expect("pending page should resolve");
        assert_eq!(selection.selected[0].id, "release-b");
        assert_eq!(
            selection.page,
            BatchPage {
                matched_tracks: 2,
                start_offset: 0,
                examined_tracks: 2,
                selected_tracks: 1,
                fully_cached_skipped: 1,
                next_offset: None,
                has_more: false,
            }
        );
        assert_eq!(
            enrich_page_summary_counts(&selection.page, 2, 1),
            (2, 4, 3),
            "the inspected page should count one no-match-complete track and the selected track's cached provider"
        );
    }

    #[test]
    fn enrich_tracks_pending_page_keeps_error_and_partial_provider_work_pending() {
        let (_dir, conn) = store();
        let tracks = vec![track("error", "Error Album")];
        let album = crate::domain::metadata::normalize_for_matching("Error Album");
        cache(&conn, "discogs", Some(&album), "error");
        cache(&conn, "bandcamp", None, "exact");

        let complete = completion(
            &conn,
            &tracks,
            &[
                crate::application::enrichment::model::EnrichmentProvider::Discogs,
                crate::application::enrichment::model::EnrichmentProvider::Bandcamp,
            ],
        )
        .expect("completion lookup should succeed");
        assert_eq!(complete, [false]);

        let selection = pending_batch_page(&tracks, 0, 1, |_| Ok(complete.clone()))
            .expect("partial-provider page should resolve");
        assert_eq!(
            enrich_page_summary_counts(&selection.page, 3, 1),
            (1, 3, 1),
            "error entries stay pending while successful provider cache hits remain counted"
        );
    }

    #[test]
    fn enrich_tracks_pending_page_force_refresh_marks_every_candidate_pending() {
        let tracks = vec![track("first", "A"), track("second", "B")];
        let selection = pending_batch_page(&tracks, 0, 1, |candidates| {
            Ok(vec![false; candidates.len()])
        })
        .expect("refresh page should resolve");
        assert_eq!(selection.selected[0].id, "first");
        assert_eq!(selection.page.next_offset, Some(1));
        assert!(selection.page.has_more);
    }

    #[test]
    fn enrich_tracks_pending_page_provider_or_cache_policy_change_requires_restart() {
        let (_dir, conn) = store();
        let tracks = vec![track("first", "A"), track("second", "B")];
        for album in ["A", "B"] {
            let normalized = crate::domain::metadata::normalize_for_matching(album);
            cache(&conn, "discogs", Some(&normalized), "none");
        }
        cache(&conn, "bandcamp", None, "none");

        let discogs_only = completion(
            &conn,
            &tracks,
            &[crate::application::enrichment::model::EnrichmentProvider::Discogs],
        )
        .expect("single-provider completion should resolve");
        assert_eq!(discogs_only, [true, true]);

        let expanded = completion(
            &conn,
            &tracks,
            &[
                crate::application::enrichment::model::EnrichmentProvider::Discogs,
                crate::application::enrichment::model::EnrichmentProvider::Bandcamp,
            ],
        )
        .expect("expanded-provider completion should resolve");
        assert_eq!(expanded, [true, true]);

        // Bandcamp's artist/title key is shared in this fixture. Delete it to
        // make both earlier candidates pending under the expanded policy.
        conn.execute(
            "DELETE FROM enrichment_cache WHERE provider = 'bandcamp'",
            [],
        )
        .expect("Bandcamp fixture should clear");
        let expanded = completion(
            &conn,
            &tracks,
            &[
                crate::application::enrichment::model::EnrichmentProvider::Discogs,
                crate::application::enrichment::model::EnrichmentProvider::Bandcamp,
            ],
        )
        .expect("expanded-provider completion should resolve");
        assert_eq!(expanded, [false, false]);

        let completion = |candidates: &[crate::domain::library::Track]| {
            Ok(candidates
                .iter()
                .map(|track| expanded[usize::from(track.id == "second")])
                .collect())
        };
        let stale_offset = pending_batch_page(&tracks, 1, 1, completion)
            .expect("changed-policy offset page should resolve");
        assert_eq!(stale_offset.selected[0].id, "second");
        let restarted = pending_batch_page(&tracks, 0, 1, completion)
            .expect("changed-policy restart should resolve");
        assert_eq!(restarted.selected[0].id, "first");

        let forced = pending_batch_page(&tracks, 0, 1, |candidates| {
            Ok(vec![false; candidates.len()])
        })
        .expect("force-refresh restart should resolve");
        assert_eq!(forced.selected[0].id, "first");
    }

    #[test]
    fn enrich_tracks_pending_page_join_failure_enumerates_provider_identities() {
        let failures = enrichment_join_failures(
            "retry-track",
            "Retry Artist",
            "Retry Title",
            &[
                crate::application::enrichment::model::EnrichmentProvider::Discogs,
                crate::application::enrichment::model::EnrichmentProvider::Bandcamp,
            ],
            "cache_writer_join",
            "sentinel join failure",
        );
        assert_eq!(failures.len(), 2);
        assert_eq!(failures[0]["track_id"], "retry-track");
        assert_eq!(failures[0]["provider"], "discogs");
        assert_eq!(failures[1]["provider"], "bandcamp");
        assert!(
            failures
                .iter()
                .all(|failure| failure["stage"] == "cache_writer_join")
        );
    }
}

#[cfg(test)]
mod cache_write_tests {
    use super::*;
    use std::future::Future;
    use std::time::Duration;

    type EnrichCacheWrite = EnrichmentCacheWrite;
    type EnrichCacheWriteMsg = CacheWriteRequest<EnrichmentCacheWrite>;
    type EnrichCacheWriterReport = EnrichmentCacheWriterReport;

    const OUTER_TIMEOUT: Duration = Duration::from_secs(5);
    const STEP_TIMEOUT: Duration = Duration::from_secs(2);

    fn test_write(title: &str) -> EnrichCacheWrite {
        EnrichCacheWrite {
            provider: crate::application::enrichment::model::EnrichmentProvider::Bandcamp,
            norm_artist: "test artist".to_string(),
            norm_title: title.to_string(),
            norm_album: None,
            match_quality: Some("none".to_string()),
            response_json: None,
        }
    }

    async fn bounded<T>(phase: &str, future: impl Future<Output = T>) -> Result<T, String> {
        tokio::time::timeout(STEP_TIMEOUT, future)
            .await
            .map_err(|_| format!("{phase} timed out"))
    }

    async fn write_enrichment_cache(
        sender: &tokio::sync::mpsc::Sender<EnrichCacheWriteMsg>,
        write: EnrichCacheWrite,
    ) -> Result<(), String> {
        acknowledge_mcp_enrichment_cache_write(sender, write).await
    }

    fn run_enrich_cache_writer(
        store_path: &str,
        receiver: tokio::sync::mpsc::Receiver<EnrichCacheWriteMsg>,
    ) -> EnrichCacheWriterReport {
        run_enrichment_cache_writer(store_path, receiver)
    }

    struct AckTaskGuard {
        sender: Option<tokio::sync::mpsc::Sender<EnrichCacheWriteMsg>>,
        writer: Option<tokio::task::JoinHandle<Result<(), String>>>,
    }

    impl AckTaskGuard {
        fn new(
            sender: tokio::sync::mpsc::Sender<EnrichCacheWriteMsg>,
            writer: tokio::task::JoinHandle<Result<(), String>>,
        ) -> Self {
            Self {
                sender: Some(sender),
                writer: Some(writer),
            }
        }

        fn sender(&self) -> &tokio::sync::mpsc::Sender<EnrichCacheWriteMsg> {
            self.sender
                .as_ref()
                .expect("ack test sender should remain until cleanup")
        }

        async fn finish(&mut self) -> Result<(), String> {
            self.sender.take();
            let mut writer = self
                .writer
                .take()
                .ok_or_else(|| "ack test writer handle is missing".to_string())?;
            match tokio::time::timeout(STEP_TIMEOUT, &mut writer).await {
                Ok(Ok(result)) => result,
                Ok(Err(error)) => Err(format!("ack test writer join failed: {error}")),
                Err(_) => {
                    writer.abort();
                    let _ = tokio::time::timeout(STEP_TIMEOUT, &mut writer).await;
                    Err("ack test writer join timed out".to_string())
                }
            }
        }
    }

    impl Drop for AckTaskGuard {
        fn drop(&mut self) {
            self.sender.take();
            if let Some(writer) = self.writer.take() {
                writer.abort();
            }
        }
    }

    async fn exercise_ack(
        acknowledgement: Option<Result<(), String>>,
    ) -> Result<Result<(), String>, String> {
        let (sender, mut receiver) = tokio::sync::mpsc::channel::<EnrichCacheWriteMsg>(1);
        let writer = tokio::spawn(async move {
            let message = bounded("ack channel receive", receiver.recv())
                .await?
                .ok_or_else(|| "ack channel closed before a message arrived".to_string())?;
            if let Some(result) = acknowledgement {
                message
                    .acknowledgement
                    .send(result)
                    .map_err(|_| "ack receiver dropped unexpectedly".to_string())?;
            }
            Ok(())
        });
        let mut guard = AckTaskGuard::new(sender, writer);
        let result = bounded(
            "oneshot acknowledgement",
            write_enrichment_cache(guard.sender(), test_write("ack test")),
        )
        .await;
        let cleanup = guard.finish().await;
        cleanup?;
        result
    }

    #[tokio::test]
    async fn enrich_cache_ack_success() {
        let result = tokio::time::timeout(OUTER_TIMEOUT, exercise_ack(Some(Ok(()))))
            .await
            .expect("ack success scenario should finish within five seconds")
            .expect("ack success harness should finish cleanly");
        assert_eq!(result, Ok(()));
    }

    #[tokio::test]
    async fn enrich_cache_ack_explicit_writer_error() {
        let result = tokio::time::timeout(
            OUTER_TIMEOUT,
            exercise_ack(Some(Err("cache write failed: sentinel".to_string()))),
        )
        .await
        .expect("ack error scenario should finish within five seconds")
        .expect("ack error harness should finish cleanly");
        assert_eq!(result, Err("cache write failed: sentinel".to_string()));
    }

    #[tokio::test]
    async fn enrich_cache_ack_closed_queue() {
        let result = tokio::time::timeout(OUTER_TIMEOUT, async {
            let (sender, receiver) = tokio::sync::mpsc::channel(1);
            drop(receiver);
            bounded(
                "closed queue send",
                write_enrichment_cache(&sender, test_write("closed queue")),
            )
            .await
        })
        .await
        .expect("closed queue scenario should finish within five seconds")
        .expect("closed queue bounded wait should finish");
        assert_eq!(result, Err("cache write queue closed".to_string()));
    }

    #[tokio::test]
    async fn enrich_cache_ack_canceled_sender() {
        let result = tokio::time::timeout(OUTER_TIMEOUT, exercise_ack(None))
            .await
            .expect("ack cancellation scenario should finish within five seconds")
            .expect("ack cancellation harness should finish cleanly");
        assert_eq!(
            result,
            Err("cache writer acknowledgement canceled".to_string())
        );
    }

    struct WriterHarness {
        sender: Option<tokio::sync::mpsc::Sender<EnrichCacheWriteMsg>>,
        writer: Option<tokio::task::JoinHandle<EnrichCacheWriterReport>>,
    }

    impl WriterHarness {
        fn new(store_path: String) -> Self {
            let (sender, receiver) = tokio::sync::mpsc::channel(2);
            let writer =
                tokio::task::spawn_blocking(move || run_enrich_cache_writer(&store_path, receiver));
            Self {
                sender: Some(sender),
                writer: Some(writer),
            }
        }

        fn sender(&self) -> &tokio::sync::mpsc::Sender<EnrichCacheWriteMsg> {
            self.sender
                .as_ref()
                .expect("writer test sender should remain until cleanup")
        }

        async fn write(&self, write: EnrichCacheWrite) -> Result<(), String> {
            bounded(
                "writer acknowledgement",
                write_enrichment_cache(self.sender(), write),
            )
            .await?
        }

        async fn send_with_dropped_ack(&self, write: EnrichCacheWrite) -> Result<(), String> {
            let (acknowledgement, acknowledged) = tokio::sync::oneshot::channel();
            drop(acknowledged);
            bounded(
                "dropped-ack channel send",
                self.sender().send(CacheWriteRequest {
                    payload: write,
                    acknowledgement,
                }),
            )
            .await?
            .map_err(|_| "writer queue closed during dropped-ack test".to_string())
        }

        async fn finish(&mut self) -> Result<EnrichCacheWriterReport, String> {
            self.sender.take();
            let mut writer = self
                .writer
                .take()
                .ok_or_else(|| "writer test handle is missing".to_string())?;
            match tokio::time::timeout(STEP_TIMEOUT, &mut writer).await {
                Ok(Ok(report)) => Ok(report),
                Ok(Err(error)) => Err(format!("writer test join failed: {error}")),
                Err(_) => {
                    writer.abort();
                    let _ = tokio::time::timeout(STEP_TIMEOUT, &mut writer).await;
                    Err("writer test join timed out".to_string())
                }
            }
        }
    }

    impl Drop for WriterHarness {
        fn drop(&mut self) {
            self.sender.take();
            if let Some(writer) = self.writer.take() {
                writer.abort();
            }
        }
    }

    #[tokio::test]
    async fn enrich_cache_writer_persists_and_reports_success() {
        let (result, report) = tokio::time::timeout(OUTER_TIMEOUT, async {
            let store_dir = tempfile::tempdir().expect("writer store dir should create");
            let store_path = store_dir.path().join("internal.sqlite3");
            let mut harness = WriterHarness::new(store_path.to_string_lossy().to_string());
            let result = harness.write(test_write("writer success")).await;
            let report = harness.finish().await?;
            Ok::<_, String>((result, report))
        })
        .await
        .expect("writer success scenario should finish within five seconds")
        .expect("writer success harness should finish cleanly");

        assert_eq!(result, Ok(()));
        assert_eq!(
            report,
            EnrichCacheWriterReport {
                attempted: 1,
                succeeded: 1,
                failed: 0,
                dropped_ack_receivers: 0,
            }
        );
    }

    #[tokio::test]
    async fn enrich_cache_writer_continues_after_selective_failure() {
        let (rejected, accepted, report) = tokio::time::timeout(OUTER_TIMEOUT, async {
            let store_dir = tempfile::tempdir().expect("writer store dir should create");
            let store_path = store_dir.path().join("internal.sqlite3");
            let conn = store::open(&store_path.to_string_lossy())
                .expect("writer test store should initialize");
            conn.execute_batch(
                "CREATE TRIGGER fail_selected_enrichment
                 BEFORE INSERT ON enrichment_cache
                 WHEN NEW.query_title = 'reject this'
                 BEGIN
                     SELECT RAISE(FAIL, 'selected cache write failure');
                 END;",
            )
            .expect("selective failure trigger should install");
            drop(conn);

            let mut harness = WriterHarness::new(store_path.to_string_lossy().to_string());
            let rejected = harness.write(test_write("reject this")).await;
            let accepted = harness.write(test_write("accept this")).await;
            let report = harness.finish().await?;
            Ok::<_, String>((rejected, accepted, report))
        })
        .await
        .expect("selective writer scenario should finish within five seconds")
        .expect("selective writer harness should finish cleanly");

        assert!(
            rejected
                .expect_err("selected cache write should fail")
                .starts_with("cache write failed:")
        );
        assert_eq!(accepted, Ok(()));
        assert_eq!(report.attempted, 2);
        assert_eq!(report.succeeded, 1);
        assert_eq!(report.failed, 1);
    }

    #[tokio::test]
    async fn enrich_cache_writer_open_failure_drains_all_messages() {
        let (first, second, report) = tokio::time::timeout(OUTER_TIMEOUT, async {
            let store_dir = tempfile::tempdir().expect("writer store dir should create");
            let mut harness = WriterHarness::new(store_dir.path().to_string_lossy().to_string());
            let first = harness.write(test_write("open failure one")).await;
            let second = harness.write(test_write("open failure two")).await;
            let report = harness.finish().await?;
            Ok::<_, String>((first, second, report))
        })
        .await
        .expect("writer-open scenario should finish within five seconds")
        .expect("writer-open harness should finish cleanly");

        for result in [first, second] {
            assert!(
                result
                    .expect_err("writer-open failure should reject every message")
                    .starts_with("cache writer open failed:")
            );
        }
        assert_eq!(report.attempted, 2);
        assert_eq!(report.succeeded, 0);
        assert_eq!(report.failed, 2);
    }

    #[tokio::test]
    async fn enrich_cache_writer_records_dropped_ack_and_continues() {
        let (next, report) = tokio::time::timeout(OUTER_TIMEOUT, async {
            let store_dir = tempfile::tempdir().expect("writer store dir should create");
            let store_path = store_dir.path().join("internal.sqlite3");
            let mut harness = WriterHarness::new(store_path.to_string_lossy().to_string());
            let dropped = harness
                .send_with_dropped_ack(test_write("dropped ack"))
                .await;
            let next = harness.write(test_write("after dropped ack")).await;
            let report = harness.finish().await?;
            dropped?;
            Ok::<_, String>((next, report))
        })
        .await
        .expect("dropped-ack scenario should finish within five seconds")
        .expect("dropped-ack harness should finish cleanly");

        assert_eq!(next, Ok(()));
        assert_eq!(report.attempted, 2);
        assert_eq!(report.succeeded, 2);
        assert_eq!(report.failed, 0);
        assert_eq!(report.dropped_ack_receivers, 1);
    }
}
