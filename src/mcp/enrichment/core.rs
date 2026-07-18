use crate::adapters::providers::discogs;
#[cfg(test)]
use crate::adapters::providers::{bandcamp, musicbrainz};
#[cfg(test)]
pub(in crate::mcp) use crate::application::enrichment::lookup::lookup_output_with_cache_metadata;

#[cfg(test)]
use std::collections::HashMap;
#[cfg(test)]
use std::sync::{Mutex, OnceLock};

#[cfg(test)]
pub(in crate::mcp) type DiscogsLookupOverrideResult =
    Result<Option<discogs::DiscogsResult>, discogs::LookupError>;
#[cfg(test)]
type DiscogsLookupOverrideKey = (String, String, Option<String>);
#[cfg(test)]
static TEST_DISCOGS_LOOKUP_OVERRIDES: OnceLock<
    Mutex<HashMap<DiscogsLookupOverrideKey, DiscogsLookupOverrideResult>>,
> = OnceLock::new();
#[cfg(test)]
pub(in crate::mcp) fn set_test_discogs_lookup_override(
    artist: &str,
    title: &str,
    album: Option<&str>,
    result: DiscogsLookupOverrideResult,
) {
    let map = TEST_DISCOGS_LOOKUP_OVERRIDES.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(mut guard) = map.lock() {
        guard.insert(
            (
                artist.to_string(),
                title.to_string(),
                album.map(str::to_string),
            ),
            result,
        );
    }
}

#[cfg(test)]
pub(in crate::mcp) fn take_test_discogs_lookup_override(
    artist: &str,
    title: &str,
    album: Option<&str>,
) -> Option<DiscogsLookupOverrideResult> {
    let map = TEST_DISCOGS_LOOKUP_OVERRIDES.get_or_init(|| Mutex::new(HashMap::new()));
    map.lock().ok()?.remove(&(
        artist.to_string(),
        title.to_string(),
        album.map(str::to_string),
    ))
}

#[cfg(test)]
type ProviderLookupOverrideKey = (String, String);
#[cfg(test)]
pub(in crate::mcp) type BandcampLookupOverrideResult =
    Result<Option<bandcamp::BandcampResult>, String>;
#[cfg(test)]
pub(in crate::mcp) type MusicBrainzLookupOverrideResult =
    Result<Option<musicbrainz::MusicBrainzResult>, String>;
#[cfg(test)]
static TEST_BANDCAMP_LOOKUP_OVERRIDES: OnceLock<
    Mutex<HashMap<ProviderLookupOverrideKey, BandcampLookupOverrideResult>>,
> = OnceLock::new();
#[cfg(test)]
static TEST_MUSICBRAINZ_LOOKUP_OVERRIDES: OnceLock<
    Mutex<HashMap<ProviderLookupOverrideKey, MusicBrainzLookupOverrideResult>>,
> = OnceLock::new();

#[cfg(test)]
pub(in crate::mcp) fn set_test_bandcamp_lookup_override(
    artist: &str,
    title: &str,
    result: BandcampLookupOverrideResult,
) {
    let map = TEST_BANDCAMP_LOOKUP_OVERRIDES.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(mut guard) = map.lock() {
        guard.insert((artist.to_string(), title.to_string()), result);
    }
}

#[cfg(test)]
pub(in crate::mcp) fn take_test_bandcamp_lookup_override(
    artist: &str,
    title: &str,
) -> Option<BandcampLookupOverrideResult> {
    let map = TEST_BANDCAMP_LOOKUP_OVERRIDES.get_or_init(|| Mutex::new(HashMap::new()));
    map.lock()
        .ok()?
        .remove(&(artist.to_string(), title.to_string()))
}

#[cfg(test)]
pub(in crate::mcp) fn set_test_musicbrainz_lookup_override(
    artist: &str,
    title: &str,
    result: MusicBrainzLookupOverrideResult,
) {
    let map = TEST_MUSICBRAINZ_LOOKUP_OVERRIDES.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(mut guard) = map.lock() {
        guard.insert((artist.to_string(), title.to_string()), result);
    }
}

#[cfg(test)]
pub(in crate::mcp) fn take_test_musicbrainz_lookup_override(
    artist: &str,
    title: &str,
) -> Option<MusicBrainzLookupOverrideResult> {
    let map = TEST_MUSICBRAINZ_LOOKUP_OVERRIDES.get_or_init(|| Mutex::new(HashMap::new()));
    map.lock()
        .ok()?
        .remove(&(artist.to_string(), title.to_string()))
}

pub(in crate::mcp) fn auth_remediation_message(remediation: &discogs::AuthRemediation) -> String {
    let mut lines = vec![remediation.message.clone()];
    if let Some(auth_url) = remediation.auth_url.as_deref() {
        lines.push(format!("Auth URL: {auth_url}"));
        lines.push(
            "Authorization requires human confirmation: ask the user to inspect this URL and \
             open it in their browser."
                .to_string(),
        );
        lines.push(
            "Never pass a broker-supplied URL through a shell or terminal command.".to_string(),
        );
    }
    if let Some(poll_interval) = remediation.poll_interval_seconds {
        lines.push(format!(
            "Poll interval if polling instead of browser: {poll_interval}s"
        ));
    }
    if let Some(expires_at) = remediation.expires_at {
        lines.push(format!("Auth session expires_at (unix): {expires_at}"));
    }
    lines.join("\n")
}
