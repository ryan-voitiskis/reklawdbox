use rmcp::ErrorData as McpError;

use super::ReklawdboxServer;
use crate::store;

pub(super) trait HasScore {
    fn score(&self) -> i32;
}

impl HasScore for crate::bandcamp::BandcampResult {
    fn score(&self) -> i32 {
        self.score
    }
}

impl HasScore for crate::musicbrainz::MusicBrainzResult {
    fn score(&self) -> i32 {
        self.score
    }
}

/// Cache a lookup result (hit or miss). Returns 1 if the result had data, 0 if miss.
pub(super) fn cache_lookup_result<T: serde::Serialize + HasScore>(
    server: &ReklawdboxServer,
    provider: &str,
    norm_artist: &str,
    norm_title: &str,
    result: Option<&T>,
) -> Result<usize, McpError> {
    let store_conn = server.cache_store_conn()?;
    match result {
        Some(r) => {
            let json = match serde_json::to_string(r) {
                Ok(j) => j,
                Err(e) => {
                    tracing::warn!(
                        provider,
                        artist = norm_artist,
                        title = norm_title,
                        "cache write skipped: serialization failed: {e}"
                    );
                    return Ok(1);
                }
            };
            let quality = if r.score() >= 90 { "exact" } else { "fuzzy" };
            if let Err(e) = store::set_enrichment(
                &store_conn,
                provider,
                norm_artist,
                norm_title,
                None,
                Some(quality),
                Some(&json),
            ) {
                tracing::warn!(
                    provider,
                    artist = norm_artist,
                    title = norm_title,
                    "cache write failed: {e}"
                );
            }
            Ok(1)
        }
        None => {
            if let Err(e) = store::set_enrichment(
                &store_conn,
                provider,
                norm_artist,
                norm_title,
                None,
                Some("none"),
                None,
            ) {
                tracing::warn!(
                    provider,
                    artist = norm_artist,
                    title = norm_title,
                    "cache write (miss marker) failed: {e}"
                );
            }
            Ok(0)
        }
    }
}
