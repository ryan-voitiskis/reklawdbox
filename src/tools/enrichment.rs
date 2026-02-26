use crate::discogs;

#[cfg(test)]
use std::collections::HashMap;
#[cfg(test)]
use std::sync::{Mutex, OnceLock};
#[cfg(test)]
use crate::beatport;

#[cfg(test)]
pub(super) type DiscogsLookupOverrideResult =
    Result<Option<discogs::DiscogsResult>, discogs::LookupError>;
#[cfg(test)]
pub(super) type BeatportLookupOverrideResult =
    Result<Option<beatport::BeatportResult>, String>;

#[cfg(test)]
type DiscogsLookupOverrideKey = (String, String, Option<String>);
#[cfg(test)]
type BeatportLookupOverrideKey = (String, String);

#[cfg(test)]
static TEST_DISCOGS_LOOKUP_OVERRIDES: OnceLock<
    Mutex<HashMap<DiscogsLookupOverrideKey, DiscogsLookupOverrideResult>>,
> = OnceLock::new();
#[cfg(test)]
static TEST_BEATPORT_LOOKUP_OVERRIDES: OnceLock<
    Mutex<HashMap<BeatportLookupOverrideKey, BeatportLookupOverrideResult>>,
> = OnceLock::new();

#[cfg(test)]
pub(super) fn set_test_discogs_lookup_override(
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
pub(super) fn take_test_discogs_lookup_override(
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
pub(super) fn set_test_beatport_lookup_override(
    artist: &str,
    title: &str,
    result: BeatportLookupOverrideResult,
) {
    let map = TEST_BEATPORT_LOOKUP_OVERRIDES.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(mut guard) = map.lock() {
        guard.insert((artist.to_string(), title.to_string()), result);
    }
}

#[cfg(test)]
pub(super) fn take_test_beatport_lookup_override(
    artist: &str,
    title: &str,
) -> Option<BeatportLookupOverrideResult> {
    let map = TEST_BEATPORT_LOOKUP_OVERRIDES.get_or_init(|| Mutex::new(HashMap::new()));
    map.lock()
        .ok()?
        .remove(&(artist.to_string(), title.to_string()))
}

pub(super) fn lookup_output_with_cache_metadata(
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

pub(super) fn auth_remediation_message(remediation: &discogs::AuthRemediation) -> String {
    let mut lines = vec![remediation.message.clone()];
    if let Some(auth_url) = remediation.auth_url.as_deref() {
        lines.push(format!("Open this URL in a browser: {auth_url}"));
    }
    if let Some(poll_interval) = remediation.poll_interval_seconds {
        lines.push(format!("Suggested poll interval: {poll_interval}s"));
    }
    if let Some(expires_at) = remediation.expires_at {
        lines.push(format!("Auth session expires_at (unix): {expires_at}"));
    }
    lines.join("\n")
}
