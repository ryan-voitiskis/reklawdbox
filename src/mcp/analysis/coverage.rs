use rmcp::ErrorData as McpError;
use rmcp::model::CallToolResult;

use crate::adapters::audio;
use crate::adapters::rekordbox as db;
use crate::adapters::state as store;
use crate::application::analysis::identity::{
    AudioCacheIdentity, audio_cache_identities_with_current_stratum_input, resolved_audio_cache_key,
};
use crate::application::classification::evidence::interpret_discogs;
use crate::domain::classification::{DiscogsMatchQuality, DiscogsReadiness};
use crate::mcp::enrichment::{
    ResolveTracksOpts, describe_resolve_scope, resolve_tracks, to_percent,
};
use crate::mcp::error::{cache_error, db_error, ok_json};
use crate::mcp::server::ReklawdboxServer;

use super::CacheCoverageParams;

pub(in crate::mcp) fn handle_cache_coverage(
    server: &ReklawdboxServer,
    params: CacheCoverageParams,
) -> Result<CallToolResult, McpError> {
    let filter_description = describe_resolve_scope(
        &params.filters,
        params.track_ids.as_deref(),
        params.playlist_id.as_deref(),
        params.max_tracks,
    );

    let (total_tracks, tracks) = {
        let conn = server.rekordbox_conn()?;
        let total_tracks = db::non_sampler_track_count(&conn).map_err(db_error)?.max(0) as usize;

        let tracks = resolve_tracks(
            &conn,
            params.track_ids.as_deref(),
            params.playlist_id.as_deref(),
            params.filters,
            params.max_tracks,
            None,
            &ResolveTracksOpts {
                default_max_tracks: None,
                max_tracks_cap: None,
                exclude_samplers: true,
            },
        )?;

        (total_tracks, tracks)
    };

    let matched_tracks = tracks.len();
    let essentia_installed = server.essentia_python_path().is_some();

    let mut stratum_cached = 0usize;
    let mut essentia_cached = 0usize;
    let mut discogs_cached = 0usize;
    let mut discogs_has_result = 0usize;
    let mut discogs_usable_genre = 0usize;
    let mut discogs_matched_unmapped = 0usize;
    let mut discogs_diagnostics = 0usize;
    let mut discogs_not_searched = 0usize;
    let mut discogs_searched_no_match = 0usize;
    let mut no_audio_analysis = 0usize;
    let mut no_enrichment = 0usize;
    let mut no_data_at_all = 0usize;
    let mut has_label = 0usize;
    let mut no_label = 0usize;
    let mut enrichment_has_label = 0usize;

    {
        let current_audio_identities = audio_cache_identities_with_current_stratum_input(
            tracks.iter().map(|track| track.file_path.as_str()),
        );
        let track_keys: Vec<_> = tracks
            .iter()
            .zip(current_audio_identities)
            .map(|(t, audio_identity)| {
                let norm_artist = crate::domain::metadata::normalize_for_matching(&t.artist);
                let norm_title = crate::domain::metadata::normalize_for_matching(&t.title);
                let norm_album = crate::domain::metadata::normalize_for_matching(&t.album);
                let audio_key = audio_identity
                    .as_ref()
                    .map(|identity| identity.cache_key.clone())
                    .unwrap_or_else(|| resolved_audio_cache_key(&t.file_path));
                (
                    norm_artist,
                    norm_title,
                    norm_album,
                    audio_key,
                    audio_identity,
                )
            })
            .collect();

        let discogs_keys: Vec<_> = track_keys
            .iter()
            .map(|(artist, title, album, _, _)| {
                ("discogs", artist.as_str(), title.as_str(), album.as_str())
            })
            .collect();

        let stratum_identities: Vec<_> = track_keys
            .iter()
            .filter_map(|(_, _, _, _, identity)| identity.as_ref()?.as_stratum_store_identity())
            .collect();
        let essentia_identities: Vec<_> = track_keys
            .iter()
            .filter_map(|(_, _, _, _, identity)| {
                identity
                    .as_ref()
                    .map(AudioCacheIdentity::as_essentia_store_identity)
            })
            .collect();

        let store = server.cache_store_conn()?;

        let discogs_map =
            store::batch_get_enrichment(&store, &discogs_keys).map_err(cache_error)?;
        let stratum_set = store::batch_fresh_audio_analysis_existence(
            &store,
            &stratum_identities,
            audio::ANALYZER_STRATUM,
            audio::STRATUM_SCHEMA_VERSION,
        )
        .map_err(cache_error)?;
        let essentia_set = store::batch_fresh_audio_analysis_existence(
            &store,
            &essentia_identities,
            audio::ANALYZER_ESSENTIA,
            audio::ESSENTIA_SCHEMA_VERSION,
        )
        .map_err(cache_error)?;

        for (idx, (norm_artist, norm_title, norm_album, audio_key, _)) in
            track_keys.iter().enumerate()
        {
            let key = (
                "discogs".to_string(),
                norm_artist.clone(),
                norm_title.clone(),
                norm_album.clone(),
            );
            let discogs = interpret_discogs(discogs_map.get(&key), &[]);
            let has_discogs = discogs.readiness != DiscogsReadiness::NotSearched;
            let has_discogs_result = matches!(
                discogs.match_quality,
                Some(DiscogsMatchQuality::Exact | DiscogsMatchQuality::Fuzzy)
            );
            let has_usable_discogs = discogs.readiness == DiscogsReadiness::UsableGenre;
            if discogs.diagnostic.is_some() {
                discogs_diagnostics += 1;
            }
            let has_stratum = stratum_set.contains(audio_key);
            let has_essentia = essentia_set.contains(audio_key);

            if has_stratum {
                stratum_cached += 1;
            }
            if has_essentia {
                essentia_cached += 1;
            }
            if has_discogs {
                discogs_cached += 1;
            }
            if has_discogs_result {
                discogs_has_result += 1;
            }
            match discogs.readiness {
                DiscogsReadiness::NotSearched => discogs_not_searched += 1,
                DiscogsReadiness::NoMatch => discogs_searched_no_match += 1,
                DiscogsReadiness::MatchedUnmapped => discogs_matched_unmapped += 1,
                DiscogsReadiness::UsableGenre => discogs_usable_genre += 1,
            }
            if !has_stratum {
                no_audio_analysis += 1;
            }
            if !has_usable_discogs {
                no_enrichment += 1;
            }
            if !has_stratum && !has_essentia && !has_usable_discogs {
                no_data_at_all += 1;
            }

            let track_has_label = !tracks[idx].label.is_empty();
            if track_has_label {
                has_label += 1;
            } else {
                no_label += 1;
                if discogs.label.is_some() {
                    enrichment_has_label += 1;
                }
            }
        }
    }

    let result = serde_json::json!({
        "scope": {
            "total_tracks": total_tracks,
            "filter_description": filter_description,
            "matched_tracks": matched_tracks,
        },
        "coverage": {
            "stratum_dsp": {
                "cached": stratum_cached,
                "percent": to_percent(stratum_cached, matched_tracks),
            },
            "essentia": {
                "cached": essentia_cached,
                "percent": to_percent(essentia_cached, matched_tracks),
                "installed": essentia_installed,
            },
            "discogs": {
                "searched": discogs_cached,
                "searched_percent": to_percent(discogs_cached, matched_tracks),
                "has_result": discogs_has_result,
                "has_result_percent": to_percent(discogs_has_result, matched_tracks),
                "usable_genre": discogs_usable_genre,
                "usable_genre_percent": to_percent(discogs_usable_genre, matched_tracks),
                "matched_unmapped": discogs_matched_unmapped,
                "diagnostics": discogs_diagnostics,
            },
        },
        "label": {
            "has_label": has_label,
            "has_label_percent": to_percent(has_label, matched_tracks),
            "no_label": no_label,
            "enrichment_has_label": enrichment_has_label,
        },
        "gaps": {
            "no_audio_analysis": no_audio_analysis,
            "no_enrichment": no_enrichment,
            "no_data_at_all": no_data_at_all,
            "discogs": {
                "not_searched": discogs_not_searched,
                "searched_no_match": discogs_searched_no_match,
                "matched_unmapped": discogs_matched_unmapped,
            },
        },
    });

    ok_json(&result)
}
