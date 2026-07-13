//! Cached enrichment resolution and track-scope rules.

use std::collections::HashMap;

use crate::adapters::state::EnrichmentCacheEntry;
use crate::domain::classification::taxonomy::{
    canonical_genre_from_alias, canonical_genre_name, label_genre, map_genre_through_taxonomy,
};

/// Selected source of tracks for an enrichment workflow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TrackScope<'a> {
    TrackIds(&'a [String]),
    Playlist(&'a str),
    Search,
}

/// Resolve the public selector precedence: explicit IDs, then playlist, then search.
pub(crate) fn track_scope<'a>(
    track_ids: Option<&'a [String]>,
    playlist_id: Option<&'a str>,
) -> TrackScope<'a> {
    if let Some(track_ids) = track_ids {
        TrackScope::TrackIds(track_ids)
    } else if let Some(playlist_id) = playlist_id {
        TrackScope::Playlist(playlist_id)
    } else {
        TrackScope::Search
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TaxonomyMapping {
    pub(crate) raw: String,
    pub(crate) maps_to: Option<String>,
    pub(crate) mapping_type: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ProviderCompleteness {
    pub(crate) discogs_cached: bool,
    pub(crate) beatport_cached: bool,
    pub(crate) discogs_has_result: bool,
    pub(crate) beatport_has_result: bool,
}

/// Parsed, source-separated provider data plus the deliberate merged fallbacks.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ResolvedProviderData {
    pub(crate) discogs: Option<serde_json::Value>,
    pub(crate) beatport: Option<serde_json::Value>,
    pub(crate) effective_label: Option<String>,
    pub(crate) label_genre: Option<&'static str>,
    pub(crate) discogs_style_mappings: Vec<TaxonomyMapping>,
    pub(crate) beatport_genre_mapping: Option<TaxonomyMapping>,
    pub(crate) discogs_mapped_genres: Vec<(String, usize)>,
    pub(crate) beatport_genre_raw: Option<String>,
    pub(crate) beatport_mapped_genre: Option<String>,
    pub(crate) completeness: ProviderCompleteness,
}

fn parse_enrichment_cache(cache: Option<&EnrichmentCacheEntry>) -> Option<serde_json::Value> {
    cache.and_then(|cached| {
        let mut value = cached
            .response_json
            .as_ref()
            .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())?;
        if let serde_json::Value::Object(ref mut map) = value {
            map.insert(
                "match_quality".into(),
                serde_json::json!(cached.match_quality),
            );
            map.insert("cached_at".into(), serde_json::json!(cached.created_at));
        }
        Some(value)
    })
}

pub(crate) fn canonical_current_genre(current_genre: &str) -> Option<&'static str> {
    if current_genre.is_empty() {
        None
    } else {
        canonical_genre_name(current_genre).or_else(|| canonical_genre_from_alias(current_genre))
    }
}

/// Resolve both full and compact provider views from the same cached source data.
pub(crate) fn resolve_cached_provider_data(
    rekordbox_label: &str,
    discogs_cache: Option<&EnrichmentCacheEntry>,
    beatport_cache: Option<&EnrichmentCacheEntry>,
) -> ResolvedProviderData {
    let discogs = parse_enrichment_cache(discogs_cache);
    let beatport = parse_enrichment_cache(beatport_cache);

    let discogs_label = discogs
        .as_ref()
        .and_then(|value| value.get("label"))
        .and_then(serde_json::Value::as_str)
        .filter(|label| !label.is_empty());
    let effective_label = if rekordbox_label.is_empty() {
        discogs_label.map(str::to_string)
    } else {
        Some(rekordbox_label.to_string())
    };
    let label_genre = effective_label.as_deref().and_then(label_genre);

    let mut discogs_genre_counts: HashMap<String, usize> = HashMap::new();
    let discogs_style_mappings = discogs
        .as_ref()
        .and_then(|value| value.get("styles"))
        .and_then(serde_json::Value::as_array)
        .map(|styles| {
            styles
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(|style| {
                    let (maps_to, mapping_type) = map_genre_through_taxonomy(style);
                    if mapping_type != "unknown"
                        && let Some(genre) = maps_to.as_ref()
                    {
                        *discogs_genre_counts.entry(genre.clone()).or_insert(0) += 1;
                    }
                    TaxonomyMapping {
                        raw: style.to_string(),
                        maps_to,
                        mapping_type,
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let mut discogs_mapped_genres: Vec<_> = discogs_genre_counts.into_iter().collect();
    discogs_mapped_genres.sort_by(|left, right| left.0.cmp(&right.0));

    let beatport_genre_raw = beatport
        .as_ref()
        .and_then(|value| value.get("genre"))
        .and_then(serde_json::Value::as_str)
        .filter(|genre| !genre.is_empty())
        .map(str::to_string);
    let beatport_genre_mapping = beatport_genre_raw.as_deref().map(|genre| {
        let (maps_to, mapping_type) = map_genre_through_taxonomy(genre);
        TaxonomyMapping {
            raw: genre.to_string(),
            maps_to,
            mapping_type,
        }
    });
    let beatport_mapped_genre = beatport_genre_mapping.as_ref().and_then(|mapping| {
        (mapping.mapping_type != "unknown")
            .then(|| mapping.maps_to.clone())
            .flatten()
    });

    ResolvedProviderData {
        completeness: ProviderCompleteness {
            discogs_cached: discogs_cache.is_some(),
            beatport_cached: beatport_cache.is_some(),
            discogs_has_result: discogs.is_some(),
            beatport_has_result: beatport.is_some(),
        },
        discogs,
        beatport,
        effective_label,
        label_genre,
        discogs_style_mappings,
        beatport_genre_mapping,
        discogs_mapped_genres,
        beatport_genre_raw,
        beatport_mapped_genre,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cached(provider: &str, response: serde_json::Value) -> EnrichmentCacheEntry {
        EnrichmentCacheEntry {
            provider: provider.to_string(),
            query_artist: "shared artist".to_string(),
            query_title: "shared title".to_string(),
            query_album: String::new(),
            match_quality: Some("exact".to_string()),
            response_json: Some(response.to_string()),
            created_at: "2026-07-14T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn explicit_track_ids_take_precedence_over_playlist_and_search() {
        let ids = vec!["track-1".to_string(), "track-2".to_string()];
        assert_eq!(
            track_scope(Some(&ids), Some("playlist-1")),
            TrackScope::TrackIds(&ids)
        );
        assert_eq!(
            track_scope(None, Some("playlist-1")),
            TrackScope::Playlist("playlist-1")
        );
        assert_eq!(track_scope(None, None), TrackScope::Search);
    }

    #[test]
    fn enrichment_resolution_preserves_provider_precedence() {
        let discogs = cached(
            "discogs",
            serde_json::json!({
                "label": "Discogs Records",
                "styles": ["Deep House", "Ambient"],
            }),
        );
        let beatport = cached(
            "beatport",
            serde_json::json!({
                "label": "Beatport Must Not Override",
                "genre": "Techno (Peak Time / Driving)",
            }),
        );

        let conflict =
            resolve_cached_provider_data("Rekordbox Records", Some(&discogs), Some(&beatport));
        assert_eq!(
            conflict.effective_label.as_deref(),
            Some("Rekordbox Records")
        );
        assert_eq!(conflict.discogs_style_mappings.len(), 2);
        assert_eq!(
            conflict.beatport_genre_raw.as_deref(),
            Some("Techno (Peak Time / Driving)")
        );
        assert!(conflict.completeness.discogs_cached);
        assert!(conflict.completeness.beatport_has_result);

        let fallback = resolve_cached_provider_data("", Some(&discogs), Some(&beatport));
        assert_eq!(fallback.effective_label.as_deref(), Some("Discogs Records"));
        assert_eq!(
            fallback.beatport_genre_raw.as_deref(),
            Some("Techno (Peak Time / Driving)"),
            "Beatport taxonomy remains source-separated and cannot override the label"
        );
        assert_eq!(fallback.discogs.as_ref().unwrap()["match_quality"], "exact");
        assert_eq!(
            fallback.beatport.as_ref().unwrap()["cached_at"],
            "2026-07-14T00:00:00Z"
        );
    }
}
