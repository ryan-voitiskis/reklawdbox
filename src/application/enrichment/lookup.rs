//! Cache-first provider lookup policy shared by MCP lookup tools.

use serde::Serialize;

use crate::adapters::{providers, state};

use super::model::{CacheLookupOutcome, ProviderLookupOutcome};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LookupProvider {
    Discogs,
    Bandcamp,
    MusicBrainz,
}

impl LookupProvider {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Discogs => "discogs",
            Self::Bandcamp => "bandcamp",
            Self::MusicBrainz => "musicbrainz",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LookupIdentity {
    pub(crate) artist: String,
    pub(crate) title: String,
    pub(crate) album: Option<String>,
    pub(crate) norm_artist: String,
    pub(crate) norm_title: String,
    pub(crate) norm_album: Option<String>,
}

impl LookupIdentity {
    pub(crate) fn new(artist: String, title: String, album: Option<String>) -> Self {
        let norm_artist = crate::domain::metadata::normalize_for_matching(&artist);
        let norm_title = crate::domain::metadata::normalize_for_matching(&title);
        let norm_album = album
            .as_deref()
            .map(crate::domain::metadata::normalize_for_matching)
            .filter(|album| !album.is_empty());
        Self {
            artist,
            title,
            album,
            norm_artist,
            norm_title,
            norm_album,
        }
    }

    fn cache_album(&self, provider: LookupProvider) -> Option<&str> {
        (provider == LookupProvider::Discogs)
            .then_some(self.norm_album.as_deref())
            .flatten()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LookupPolicy {
    pub(crate) force_refresh: bool,
    pub(crate) cache_read_enabled: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LookupResult {
    pub(crate) payload: serde_json::Value,
    pub(crate) cache_hit: bool,
    pub(crate) cached_at: Option<String>,
}

impl LookupResult {
    pub(crate) fn into_output(self) -> serde_json::Value {
        lookup_output_with_cache_metadata(self.payload, self.cache_hit, self.cached_at.as_deref())
    }
}

pub(crate) fn read_lookup_cache(
    conn: &rusqlite::Connection,
    provider: LookupProvider,
    identity: &LookupIdentity,
    policy: LookupPolicy,
) -> Result<CacheLookupOutcome<LookupResult>, rusqlite::Error> {
    if policy.force_refresh || !policy.cache_read_enabled {
        return Ok(CacheLookupOutcome::Miss);
    }

    let cached = state::get_enrichment(
        conn,
        provider.as_str(),
        &identity.norm_artist,
        &identity.norm_title,
        identity.cache_album(provider),
        false,
    )?;
    Ok(match cached {
        Some(cached) => {
            let payload = cached
                .response_json
                .as_deref()
                .and_then(|json| serde_json::from_str(json).ok())
                .unwrap_or(serde_json::Value::Null);
            CacheLookupOutcome::Hit(LookupResult {
                payload,
                cache_hit: true,
                cached_at: Some(cached.created_at),
            })
        }
        None => CacheLookupOutcome::Miss,
    })
}

#[derive(Debug)]
pub(crate) enum PersistLookupError {
    Cache(rusqlite::Error),
    Serialize(serde_json::Error),
}

fn persist_provider_result<T: Serialize>(
    conn: &rusqlite::Connection,
    provider: LookupProvider,
    identity: &LookupIdentity,
    outcome: ProviderLookupOutcome<T>,
    match_quality: impl FnOnce(&T) -> &'static str,
) -> Result<LookupResult, PersistLookupError> {
    let (payload, quality, response_json) = match outcome {
        ProviderLookupOutcome::Match(result) => {
            let quality = match_quality(&result).to_string();
            let response_json =
                serde_json::to_string(&result).map_err(PersistLookupError::Serialize)?;
            let payload = serde_json::to_value(result).map_err(PersistLookupError::Serialize)?;
            (payload, quality, Some(response_json))
        }
        ProviderLookupOutcome::NoMatch => (serde_json::Value::Null, "none".to_string(), None),
    };

    state::set_enrichment(
        conn,
        provider.as_str(),
        &identity.norm_artist,
        &identity.norm_title,
        identity.cache_album(provider),
        Some(&quality),
        response_json.as_deref(),
    )
    .map_err(PersistLookupError::Cache)?;
    Ok(LookupResult {
        payload,
        cache_hit: false,
        cached_at: None,
    })
}

fn provider_outcome<T>(result: Option<T>) -> ProviderLookupOutcome<T> {
    match result {
        Some(result) => ProviderLookupOutcome::Match(result),
        None => ProviderLookupOutcome::NoMatch,
    }
}

pub(crate) fn persist_discogs_result(
    conn: &rusqlite::Connection,
    identity: &LookupIdentity,
    result: Option<providers::discogs::DiscogsResult>,
) -> Result<LookupResult, PersistLookupError> {
    persist_provider_result(
        conn,
        LookupProvider::Discogs,
        identity,
        provider_outcome(result),
        |result| {
            if result.fuzzy_match { "fuzzy" } else { "exact" }
        },
    )
}

pub(crate) async fn dispatch_bandcamp(
    http: &reqwest::Client,
    identity: &LookupIdentity,
    url: Option<&str>,
) -> Result<Option<providers::bandcamp::BandcampResult>, providers::bandcamp::BandcampError> {
    match url {
        Some(url) => {
            providers::bandcamp::lookup_url(http, url, &identity.artist, &identity.title).await
        }
        None => providers::bandcamp::lookup(http, &identity.artist, &identity.title).await,
    }
}

pub(crate) fn persist_bandcamp_result(
    conn: &rusqlite::Connection,
    identity: &LookupIdentity,
    result: Option<providers::bandcamp::BandcampResult>,
) -> Result<LookupResult, PersistLookupError> {
    persist_provider_result(
        conn,
        LookupProvider::Bandcamp,
        identity,
        provider_outcome(result),
        |result| {
            if result.score == 100 {
                "exact"
            } else {
                "fuzzy"
            }
        },
    )
}

pub(crate) async fn dispatch_musicbrainz(
    http: &reqwest::Client,
    identity: &LookupIdentity,
) -> Result<
    Option<providers::musicbrainz::MusicBrainzResult>,
    providers::musicbrainz::MusicBrainzError,
> {
    providers::musicbrainz::lookup(http, &identity.artist, &identity.title).await
}

pub(crate) fn persist_musicbrainz_result(
    conn: &rusqlite::Connection,
    identity: &LookupIdentity,
    result: Option<providers::musicbrainz::MusicBrainzResult>,
) -> Result<LookupResult, PersistLookupError> {
    persist_provider_result(
        conn,
        LookupProvider::MusicBrainz,
        identity,
        provider_outcome(result),
        |result| {
            if result.score >= 100 {
                "exact"
            } else {
                "fuzzy"
            }
        },
    )
}

pub(crate) fn lookup_output_with_cache_metadata(
    payload: serde_json::Value,
    cache_hit: bool,
    cached_at: Option<&str>,
) -> serde_json::Value {
    match payload {
        serde_json::Value::Object(mut map) => {
            map.insert("cache_hit".to_string(), serde_json::json!(cache_hit));
            if let Some(cached_at) = cached_at {
                map.insert("cached_at".to_string(), serde_json::json!(cached_at));
            }
            serde_json::Value::Object(map)
        }
        other => {
            let mut map = serde_json::Map::new();
            map.insert("result".to_string(), other);
            map.insert("cache_hit".to_string(), serde_json::json!(cache_hit));
            if let Some(cached_at) = cached_at {
                map.insert("cached_at".to_string(), serde_json::json!(cached_at));
            }
            serde_json::Value::Object(map)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enrichment_lookup_preserves_negative_cache_policy() {
        let store_dir = tempfile::tempdir().expect("lookup store directory should create");
        let store_path = store_dir.path().join("internal.sqlite3");
        let conn = state::open(&store_path.to_string_lossy()).expect("lookup store should open");
        let identity = LookupIdentity::new(
            "Shared Artist".to_string(),
            "Shared Title".to_string(),
            None,
        );
        state::set_enrichment(
            &conn,
            "bandcamp",
            &identity.norm_artist,
            &identity.norm_title,
            None,
            Some("none"),
            None,
        )
        .expect("negative lookup should persist");

        let outcome = read_lookup_cache(
            &conn,
            LookupProvider::Bandcamp,
            &identity,
            LookupPolicy {
                force_refresh: false,
                cache_read_enabled: true,
            },
        )
        .expect("negative lookup should read");
        let CacheLookupOutcome::Hit(result) = outcome else {
            panic!("durable negative lookup must be a cache hit");
        };
        assert_eq!(result.payload, serde_json::Value::Null);
        assert!(result.cache_hit);

        assert!(matches!(
            read_lookup_cache(
                &conn,
                LookupProvider::Bandcamp,
                &identity,
                LookupPolicy {
                    force_refresh: true,
                    cache_read_enabled: true,
                },
            )
            .expect("forced lookup policy should resolve"),
            CacheLookupOutcome::Miss
        ));
    }
}
