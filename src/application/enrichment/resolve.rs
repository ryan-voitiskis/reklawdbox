//! Enrichment track-scope resolution rules.

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

/// Source-separated provider fields plus the one merged fallback field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProviderResolution<'a> {
    pub(crate) effective_label: Option<&'a str>,
    pub(crate) discogs_styles: Vec<&'a str>,
    pub(crate) beatport_genre: Option<&'a str>,
}

/// Preserve the existing resolution hierarchy without merging provider taxonomies.
pub(crate) fn resolve_provider_data<'a>(
    rekordbox_label: &'a str,
    discogs: Option<&'a serde_json::Value>,
    beatport: Option<&'a serde_json::Value>,
) -> ProviderResolution<'a> {
    let discogs_label = discogs
        .and_then(|value| value.get("label"))
        .and_then(serde_json::Value::as_str)
        .filter(|label| !label.is_empty());
    let effective_label = if rekordbox_label.is_empty() {
        discogs_label
    } else {
        Some(rekordbox_label)
    };
    let discogs_styles = discogs
        .and_then(|value| value.get("styles"))
        .and_then(serde_json::Value::as_array)
        .map(|styles| {
            styles
                .iter()
                .filter_map(serde_json::Value::as_str)
                .collect()
        })
        .unwrap_or_default();
    let beatport_genre = beatport
        .and_then(|value| value.get("genre"))
        .and_then(serde_json::Value::as_str)
        .filter(|genre| !genre.is_empty());

    ProviderResolution {
        effective_label,
        discogs_styles,
        beatport_genre,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let discogs = serde_json::json!({
            "label": "Discogs Records",
            "styles": ["Deep House", "Ambient"],
        });
        let beatport = serde_json::json!({
            "label": "Beatport Must Not Override",
            "genre": "Techno (Peak Time / Driving)",
        });

        let conflict = resolve_provider_data("Rekordbox Records", Some(&discogs), Some(&beatport));
        assert_eq!(conflict.effective_label, Some("Rekordbox Records"));
        assert_eq!(conflict.discogs_styles, ["Deep House", "Ambient"]);
        assert_eq!(
            conflict.beatport_genre,
            Some("Techno (Peak Time / Driving)")
        );

        let fallback = resolve_provider_data("", Some(&discogs), Some(&beatport));
        assert_eq!(fallback.effective_label, Some("Discogs Records"));
        assert_eq!(
            fallback.beatport_genre,
            Some("Techno (Peak Time / Driving)"),
            "Beatport taxonomy remains source-separated and cannot override the label"
        );
    }
}
