use std::sync::OnceLock;

use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex as TokioMutex;
use tokio::time::Instant;

use crate::discogs::urlencoding;

static RATE_LIMITER: OnceLock<TokioMutex<Option<Instant>>> = OnceLock::new();

const USER_AGENT: &str = concat!(
    "reklawdbox/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/ryan-voitiskis/reklawdbox)"
);

#[derive(Debug, thiserror::Error)]
pub enum MusicBrainzError {
    #[error("MusicBrainz HTTP {status}")]
    Http {
        status: reqwest::StatusCode,
        body: String,
        retry_after: Option<String>,
    },
    #[error("{0}")]
    Parse(String),
    #[error("{0}")]
    Request(#[from] reqwest::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MusicBrainzResult {
    pub recording_title: String,
    pub artist: String,
    pub first_release_date: Option<String>,
    pub label: Option<String>,
    pub score: i32,
}

async fn wait_for_rate_limit() {
    let last = RATE_LIMITER.get_or_init(|| TokioMutex::new(None));
    crate::rate_limit::wait(last, "REKLAWDBOX_MUSICBRAINZ_MIN_INTERVAL_MS", 1100).await;
}

pub async fn lookup(
    client: &Client,
    artist: &str,
    title: &str,
) -> Result<Option<MusicBrainzResult>, MusicBrainzError> {
    wait_for_rate_limit().await;

    // Recording search
    let query = format!("artist:\"{}\" AND recording:\"{}\"", artist, title);
    let url = format!(
        "https://musicbrainz.org/ws/2/recording/?query={}&fmt=json&limit=5",
        urlencoding(&query)
    );

    let resp = client
        .get(&url)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/json")
        .send()
        .await?;

    let status = resp.status();
    if !status.is_success() {
        let retry_after = resp
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let body = resp.text().await.unwrap_or_default();
        return Err(MusicBrainzError::Http {
            status,
            body,
            retry_after,
        });
    }

    let body = resp.text().await?;
    let search_result: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| MusicBrainzError::Parse(format!("Recording search JSON parse error: {e}")))?;

    let recordings = match search_result.get("recordings") {
        Some(v) => match v.as_array() {
            Some(arr) => arr,
            None => {
                return Err(MusicBrainzError::Parse(
                    "MusicBrainz 'recordings' is not an array".into(),
                ));
            }
        },
        None => {
            return Err(MusicBrainzError::Parse(
                "MusicBrainz response missing 'recordings' key".into(),
            ));
        }
    };

    if recordings.is_empty() {
        return Ok(None);
    }

    // Find best recording with score >= 90
    let best = recordings.iter().find(|r| {
        r.get("score")
            .and_then(|s| s.as_i64())
            .is_some_and(|s| s >= 90)
    });

    let Some(recording) = best else {
        return Ok(None);
    };

    let score = recording.get("score").and_then(|s| s.as_i64()).unwrap_or(0) as i32;

    let recording_title = recording
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let recording_artist = recording
        .get("artist-credit")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .and_then(|a| a.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or(artist)
        .to_string();

    let first_release_date = recording
        .get("first-release-date")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // Pick best release ID for label lookup
    let releases = recording.get("releases").and_then(|v| v.as_array());

    let best_release_id = releases.and_then(|rels| {
        let mut scored: Vec<(&serde_json::Value, i32)> =
            rels.iter().map(|r| (r, score_release(r))).collect();
        scored.sort_by(|a, b| b.1.cmp(&a.1));
        scored
            .first()
            .and_then(|(r, _)| r.get("id").and_then(|v| v.as_str()))
            .map(|s| s.to_string())
    });

    // Look up release for label — degrade gracefully on failure
    let label = if let Some(ref release_id) = best_release_id {
        wait_for_rate_limit().await;
        match lookup_release_label(client, release_id).await {
            Ok(l) => l,
            Err(e) => {
                tracing::warn!(
                    release_id,
                    "MusicBrainz label lookup failed, returning without label: {e}"
                );
                None
            }
        }
    } else {
        None
    };

    Ok(Some(MusicBrainzResult {
        recording_title,
        artist: recording_artist,
        first_release_date,
        label,
        score,
    }))
}

async fn lookup_release_label(
    client: &Client,
    release_id: &str,
) -> Result<Option<String>, MusicBrainzError> {
    let url = format!(
        "https://musicbrainz.org/ws/2/release/{}?inc=labels&fmt=json",
        release_id
    );

    let resp = client
        .get(&url)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/json")
        .send()
        .await?;

    let status = resp.status();
    if !status.is_success() {
        let retry_after = resp
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let body = resp.text().await.unwrap_or_default();
        return Err(MusicBrainzError::Http {
            status,
            body,
            retry_after,
        });
    }

    let body = resp.text().await?;
    let release: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| MusicBrainzError::Parse(format!("Release lookup JSON parse error: {e}")))?;

    let label = release
        .get("label-info")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .and_then(|li| li.get("label"))
        .and_then(|l| l.get("name"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // Filter out "[no label]"
    let label = label.filter(|l| l != "[no label]" && !l.is_empty());

    Ok(label)
}

/// Score a release for picking the best one from a recording's releases array.
///
/// Prefer Single/EP over Album, exclude compilations/DJ-mixes/live,
/// prefer Official status, earliest date breaks ties.
fn score_release(release: &serde_json::Value) -> i32 {
    let mut score = 0i32;

    // Release group primary type scoring
    let primary_type = release
        .get("release-group")
        .and_then(|rg| rg.get("primary-type"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    match primary_type {
        "Single" => score += 30,
        "EP" => score += 25,
        "Album" => score += 10,
        _ => score += 5,
    }

    // Penalise secondary types: Compilation, DJ-mix, Live
    let secondary_types = release
        .get("release-group")
        .and_then(|rg| rg.get("secondary-types"))
        .and_then(|v| v.as_array());

    if let Some(types) = secondary_types {
        for t in types {
            if let Some("Compilation" | "DJ-mix" | "Live") = t.as_str() {
                score -= 50;
            }
        }
    }

    // Prefer Official status
    let status = release.get("status").and_then(|v| v.as_str()).unwrap_or("");

    if status == "Official" {
        score += 15;
    }

    // Prefer earlier dates (subtract year as a tiebreaker)
    let date = release
        .get("date")
        .and_then(|v| v.as_str())
        .unwrap_or("9999");

    if let Some(year_str) = date.get(..4)
        && let Ok(year) = year_str.parse::<i32>()
    {
        // Subtract a small amount so earlier years score higher
        // Use 2100 - year so year 2000 → +100, year 2020 → +80
        score += (2100 - year).clamp(0, 200);
    }

    score
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn score_release_prefers_single_over_album() {
        let single = serde_json::json!({
            "release-group": { "primary-type": "Single" },
            "status": "Official",
            "date": "2019-08-03"
        });
        let album = serde_json::json!({
            "release-group": { "primary-type": "Album" },
            "status": "Official",
            "date": "2019-08-03"
        });
        assert!(score_release(&single) > score_release(&album));
    }

    #[test]
    fn score_release_penalises_compilation() {
        let normal = serde_json::json!({
            "release-group": { "primary-type": "Album" },
            "status": "Official",
            "date": "2019"
        });
        let compilation = serde_json::json!({
            "release-group": {
                "primary-type": "Album",
                "secondary-types": ["Compilation"]
            },
            "status": "Official",
            "date": "2019"
        });
        assert!(score_release(&normal) > score_release(&compilation));
    }

    #[test]
    fn score_release_prefers_official() {
        let official = serde_json::json!({
            "release-group": { "primary-type": "Single" },
            "status": "Official",
            "date": "2019"
        });
        let promo = serde_json::json!({
            "release-group": { "primary-type": "Single" },
            "status": "Promotion",
            "date": "2019"
        });
        assert!(score_release(&official) > score_release(&promo));
    }

    #[test]
    fn score_release_prefers_earlier_date() {
        let earlier = serde_json::json!({
            "release-group": { "primary-type": "Single" },
            "status": "Official",
            "date": "2010-01-01"
        });
        let later = serde_json::json!({
            "release-group": { "primary-type": "Single" },
            "status": "Official",
            "date": "2020-06-15"
        });
        assert!(score_release(&earlier) > score_release(&later));
    }

    #[test]
    fn score_release_dj_mix_heavily_penalised() {
        let normal = serde_json::json!({
            "release-group": { "primary-type": "Album" },
            "status": "Official",
            "date": "2020"
        });
        let dj_mix = serde_json::json!({
            "release-group": {
                "primary-type": "Album",
                "secondary-types": ["DJ-mix"]
            },
            "status": "Official",
            "date": "2020"
        });
        assert!(score_release(&normal) > score_release(&dj_mix));
    }

    #[test]
    fn parse_recording_search_response() {
        let json = serde_json::json!({
            "recordings": [
                {
                    "score": 100,
                    "title": "Archangel",
                    "artist-credit": [{"name": "Burial"}],
                    "first-release-date": "2007-11-12",
                    "releases": [
                        {
                            "id": "abc-123",
                            "release-group": { "primary-type": "Album" },
                            "status": "Official",
                            "date": "2007-11-12"
                        }
                    ]
                }
            ]
        });

        let recordings = json.get("recordings").unwrap().as_array().unwrap();
        let best = recordings.iter().find(|r| {
            r.get("score")
                .and_then(|s| s.as_i64())
                .is_some_and(|s| s >= 90)
        });

        assert!(best.is_some());
        let recording = best.unwrap();
        assert_eq!(
            recording.get("title").unwrap().as_str().unwrap(),
            "Archangel"
        );
        assert_eq!(
            recording
                .get("first-release-date")
                .unwrap()
                .as_str()
                .unwrap(),
            "2007-11-12"
        );
    }

    #[test]
    fn parse_recording_no_match_below_threshold() {
        let json = serde_json::json!({
            "recordings": [
                {
                    "score": 75,
                    "title": "Something Else",
                    "artist-credit": [{"name": "Unknown"}],
                    "first-release-date": "2020-01-01",
                    "releases": []
                }
            ]
        });

        let recordings = json.get("recordings").unwrap().as_array().unwrap();
        let best = recordings.iter().find(|r| {
            r.get("score")
                .and_then(|s| s.as_i64())
                .is_some_and(|s| s >= 90)
        });

        assert!(best.is_none());
    }

    #[test]
    fn year_extraction_from_first_release_date() {
        for (input, expected) in [
            ("2019-08-03", "2019-08-03"),
            ("2007", "2007"),
            ("2023-12", "2023-12"),
        ] {
            let recording = serde_json::json!({
                "first-release-date": input
            });
            let year = recording
                .get("first-release-date")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            assert_eq!(year.as_deref(), Some(expected));
        }
    }

    #[test]
    fn label_filters_no_label() {
        let labels_to_filter = ["[no label]", ""];
        for label in &labels_to_filter {
            let filtered = Some(label.to_string()).filter(|l| l != "[no label]" && !l.is_empty());
            assert!(filtered.is_none(), "should filter out: {label:?}");
        }
    }

    #[test]
    fn label_passes_valid() {
        let label = "Giegling";
        let filtered = Some(label.to_string()).filter(|l| l != "[no label]" && !l.is_empty());
        assert_eq!(filtered.as_deref(), Some("Giegling"));
    }

    #[tokio::test]
    async fn rate_limiter_enforces_minimum_spacing() {
        // Use a short interval so the test completes quickly.
        // SAFETY: test runs sequentially (no other threads reading this env var).
        unsafe { std::env::set_var("REKLAWDBOX_MUSICBRAINZ_MIN_INTERVAL_MS", "50") };

        let n = 4;
        let start = Instant::now();

        for _ in 0..n {
            wait_for_rate_limit().await;
        }

        let elapsed = start.elapsed();
        let min_expected = Duration::from_millis(50 * (n - 1));
        assert!(
            elapsed >= min_expected,
            "expected >= {min_expected:?}, got {elapsed:?}"
        );

        // Reset stored instant so it doesn't leak into other tests.
        *RATE_LIMITER.get().unwrap().lock().await = None;

        // SAFETY: cleaning up test-only env var.
        unsafe { std::env::remove_var("REKLAWDBOX_MUSICBRAINZ_MIN_INTERVAL_MS") };
    }
}
