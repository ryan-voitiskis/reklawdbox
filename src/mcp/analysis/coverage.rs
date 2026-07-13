use rmcp::ErrorData as McpError;
use rmcp::model::CallToolResult;

use crate::audio;
use crate::db;
use crate::mcp::enrichment::{
    ResolveTracksOpts, describe_resolve_scope, resolve_tracks, to_percent,
};
use crate::mcp::error::{cache_error, db_error, ok_json};
use crate::mcp::server::ReklawdboxServer;
use crate::store;

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
    let mut beatport_cached = 0usize;
    let mut beatport_has_result = 0usize;
    let mut no_audio_analysis = 0usize;
    let mut no_enrichment = 0usize;
    let mut no_data_at_all = 0usize;
    let mut has_label = 0usize;
    let mut no_label = 0usize;
    let mut enrichment_has_label = 0usize;

    {
        let current_audio_identities =
            crate::mcp::analysis::audio_cache_identities_with_current_stratum_input(
                tracks.iter().map(|track| track.file_path.as_str()),
            );
        let track_keys: Vec<_> = tracks
            .iter()
            .zip(current_audio_identities)
            .map(|(t, audio_identity)| {
                let norm_artist = crate::normalize::normalize_for_matching(&t.artist);
                let norm_title = crate::normalize::normalize_for_matching(&t.title);
                let audio_key = audio_identity
                    .as_ref()
                    .map(|identity| identity.cache_key.clone())
                    .unwrap_or_else(|| {
                        crate::mcp::analysis::resolved_audio_cache_key(&t.file_path)
                    });
                (norm_artist, norm_title, audio_key, audio_identity)
            })
            .collect();

        let unique_artists: Vec<&str> = {
            let mut seen = std::collections::HashSet::new();
            track_keys
                .iter()
                .filter_map(|(a, _, _, _)| {
                    if seen.insert(a.as_str()) {
                        Some(a.as_str())
                    } else {
                        None
                    }
                })
                .collect()
        };

        let stratum_identities: Vec<_> = track_keys
            .iter()
            .filter_map(|(_, _, _, identity)| identity.as_ref()?.as_stratum_store_identity())
            .collect();
        let essentia_identities: Vec<_> = track_keys
            .iter()
            .filter_map(|(_, _, _, identity)| {
                identity
                    .as_ref()
                    .map(crate::mcp::analysis::AudioCacheIdentity::as_essentia_store_identity)
            })
            .collect();

        let store = server.cache_store_conn()?;

        let discogs_set = store::batch_enrichment_existence(&store, "discogs", &unique_artists)
            .map_err(cache_error)?;
        let beatport_set = store::batch_enrichment_existence(&store, "beatport", &unique_artists)
            .map_err(cache_error)?;
        let discogs_result_set =
            store::batch_enrichment_with_results(&store, "discogs", &unique_artists)
                .map_err(cache_error)?;
        let beatport_result_set =
            store::batch_enrichment_with_results(&store, "beatport", &unique_artists)
                .map_err(cache_error)?;
        let discogs_label_set =
            store::batch_enrichment_with_label(&store, "discogs", &unique_artists)
                .map_err(cache_error)?;
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

        // Build borrowed-key sets to avoid per-track clones during counting.
        let discogs_ref: std::collections::HashSet<(&str, &str)> = discogs_set
            .iter()
            .map(|(a, t)| (a.as_str(), t.as_str()))
            .collect();
        let beatport_ref: std::collections::HashSet<(&str, &str)> = beatport_set
            .iter()
            .map(|(a, t)| (a.as_str(), t.as_str()))
            .collect();
        let discogs_result_ref: std::collections::HashSet<(&str, &str)> = discogs_result_set
            .iter()
            .map(|(a, t)| (a.as_str(), t.as_str()))
            .collect();
        let beatport_result_ref: std::collections::HashSet<(&str, &str)> = beatport_result_set
            .iter()
            .map(|(a, t)| (a.as_str(), t.as_str()))
            .collect();
        let discogs_label_ref: std::collections::HashSet<(&str, &str)> = discogs_label_set
            .iter()
            .map(|(a, t)| (a.as_str(), t.as_str()))
            .collect();
        for (idx, (norm_artist, norm_title, audio_key, _)) in track_keys.iter().enumerate() {
            let key = (norm_artist.as_str(), norm_title.as_str());
            let has_discogs = discogs_ref.contains(&key);
            let has_beatport = beatport_ref.contains(&key);
            let has_discogs_result = discogs_result_ref.contains(&key);
            let has_beatport_result = beatport_result_ref.contains(&key);
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
            if has_beatport {
                beatport_cached += 1;
            }
            if has_beatport_result {
                beatport_has_result += 1;
            }
            if !has_stratum {
                no_audio_analysis += 1;
            }
            if !has_discogs_result && !has_beatport_result {
                no_enrichment += 1;
            }
            if !has_stratum && !has_essentia && !has_discogs_result && !has_beatport_result {
                no_data_at_all += 1;
            }

            let track_has_label = !tracks[idx].label.is_empty();
            if track_has_label {
                has_label += 1;
            } else {
                no_label += 1;
                if discogs_label_ref.contains(&key) {
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
            },
            "beatport": {
                "searched": beatport_cached,
                "searched_percent": to_percent(beatport_cached, matched_tracks),
                "has_result": beatport_has_result,
                "has_result_percent": to_percent(beatport_has_result, matched_tracks),
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
        },
    });

    ok_json(&result)
}
