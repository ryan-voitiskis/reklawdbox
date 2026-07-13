use reqwest::Client;
use serde::{Deserialize, Serialize};

use super::http::urlencoding;

super::rate_limit::define_rate_limiter!("REKLAWDBOX_BEATPORT_MIN_INTERVAL_MS", 1000);

const BEATPORT_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 \
    (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

#[derive(Debug, thiserror::Error)]
pub enum BeatportError {
    #[error("Beatport {kind} HTTP {status}{}", .retry_after.as_ref().filter(|r| !r.is_empty()).map(|r| format!(" (Retry-After: {r})")).unwrap_or_default())]
    Http {
        status: reqwest::StatusCode,
        retry_after: Option<String>,
        kind: String,
    },
    #[error("{0}")]
    Request(#[from] reqwest::Error),
    #[error("{0}")]
    Parse(String),
}

enum HttpStatusOutcome {
    NoMatch,
    Error(BeatportError),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeatportResult {
    pub genre: String,
    pub bpm: Option<i32>,
    pub key: String,
    pub track_name: String,
    pub artists: Vec<String>,
    pub release_date: Option<String>,
    pub label: Option<String>,
    #[serde(default)]
    pub fuzzy_match: bool,
}

pub async fn lookup(
    client: &Client,
    artist: &str,
    title: &str,
) -> Result<Option<BeatportResult>, BeatportError> {
    wait_for_rate_limit().await;

    let query = format!("{artist} {title}");
    let url = format!(
        "https://www.beatport.com/search/tracks?q={}",
        urlencoding(&query)
    );

    let resp = client
        .get(&url)
        .header("User-Agent", BEATPORT_USER_AGENT)
        .header(
            "Accept",
            "text/html,application/xhtml+xml,application/xml;q=0.9,image/webp,*/*;q=0.8",
        )
        .header("Accept-Language", "en-US,en;q=0.5")
        .header("Cache-Control", "no-cache")
        .header("Pragma", "no-cache")
        .send()
        .await?;

    let status = resp.status();
    if !status.is_success() {
        let retry_after = super::rate_limit::extract_retry_after(resp.headers());
        return match classify_http_status(status, retry_after.as_deref()) {
            HttpStatusOutcome::NoMatch => Ok(None),
            HttpStatusOutcome::Error(e) => Err(e),
        };
    }

    let html = resp.text().await?;

    parse_beatport_html(&html, artist, title)
}

fn classify_http_status(
    status: reqwest::StatusCode,
    retry_after: Option<&str>,
) -> HttpStatusOutcome {
    if status == reqwest::StatusCode::NOT_FOUND {
        return HttpStatusOutcome::NoMatch;
    }

    if status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
        return HttpStatusOutcome::Error(http_status_error(
            status,
            retry_after,
            "transient/retryable",
        ));
    }

    if status.is_client_error() {
        return HttpStatusOutcome::Error(http_status_error(status, retry_after, "client"));
    }

    HttpStatusOutcome::Error(http_status_error(status, retry_after, "unexpected"))
}

fn http_status_error(
    status: reqwest::StatusCode,
    retry_after: Option<&str>,
    kind: &str,
) -> BeatportError {
    BeatportError::Http {
        status,
        retry_after: retry_after.filter(|r| !r.is_empty()).map(str::to_string),
        kind: kind.to_string(),
    }
}

fn parse_beatport_html(
    html: &str,
    artist: &str,
    title: &str,
) -> Result<Option<BeatportResult>, BeatportError> {
    let json_str = match extract_next_data_json(html) {
        Some(v) => v,
        None => {
            return Err(BeatportError::Parse(
                "Beatport HTML missing __NEXT_DATA__ script tag".to_string(),
            ));
        }
    };
    let next_data: serde_json::Value = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(e) => {
            return Err(BeatportError::Parse(format!(
                "Beatport __NEXT_DATA__ JSON malformed: {e}"
            )));
        }
    };

    let queries = match next_data.pointer("/props/pageProps/dehydratedState/queries") {
        Some(v) => v,
        None => {
            return Err(BeatportError::Parse(
                "Beatport JSON missing dehydratedState/queries path".to_string(),
            ));
        }
    };
    let queries = match queries.as_array() {
        Some(arr) => arr,
        None => {
            return Err(BeatportError::Parse(
                "Beatport queries field is not an array".to_string(),
            ));
        }
    };

    for query in queries {
        let Some(tracks) = query.pointer("/state/data/data").and_then(|v| v.as_array()) else {
            continue;
        };

        for track in tracks {
            if is_track_match(track, artist, title) {
                let track_name = track
                    .get("track_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let artists = track
                    .get("artists")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|a| a.get("artist_name").and_then(|n| n.as_str()))
                            .map(str::to_string)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let genre = track
                    .get("genre")
                    .and_then(|v| v.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|g| g.get("genre_name"))
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string();

                let bpm = track
                    .get("bpm")
                    .and_then(serde_json::Value::as_i64)
                    .and_then(|v| i32::try_from(v).ok());

                let key = track
                    .get("key_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                let release_date = track
                    .get("publish_date")
                    .or_else(|| track.get("release_date"))
                    .and_then(|v| v.as_str())
                    .map(std::string::ToString::to_string);

                let label = track
                    .get("label")
                    .and_then(|v| v.get("label_name"))
                    .and_then(|v| v.as_str())
                    .filter(|l| !l.is_empty())
                    .map(std::string::ToString::to_string);

                let fuzzy_match = !track_name.eq_ignore_ascii_case(title.trim());
                return Ok(Some(BeatportResult {
                    genre,
                    bpm,
                    key,
                    track_name: track_name.to_string(),
                    artists,
                    release_date,
                    label,
                    fuzzy_match,
                }));
            }
        }
    }

    Ok(None)
}

fn extract_next_data_json(html: &str) -> Option<&str> {
    let id_pos = html
        .find("id=\"__NEXT_DATA__\"")
        .or_else(|| html.find("id='__NEXT_DATA__'"))?;

    let script_start = html[..id_pos].rfind("<script")?;
    let open_tag_end = html[script_start..].find('>')? + script_start + 1;
    let script_end = html[open_tag_end..].find("</script>")? + open_tag_end;

    Some(html[open_tag_end..script_end].trim())
}

fn is_track_match(track: &serde_json::Value, artist: &str, title: &str) -> bool {
    let norm_artist = artist.trim().to_lowercase();
    let norm_title = title.trim().to_lowercase();
    if norm_artist.is_empty() || norm_title.is_empty() {
        return false;
    }

    let bp_artists: Vec<String> = track
        .get("artists")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|a| a.get("artist_name").and_then(|n| n.as_str()))
                .map(|name| name.trim().to_lowercase())
                .collect()
        })
        .unwrap_or_default();

    // Rekordbox stores multi-artist as a combined string ("Burial, Four Tet"),
    // but Beatport returns separate artist objects.
    let search_parts: Vec<&str> = norm_artist
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();

    let artist_match = bp_artists
        .iter()
        .any(|bp| search_parts.iter().any(|part| bp == part));

    let track_name = track
        .get("track_name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_lowercase();
    if track_name.is_empty() {
        return false;
    }
    let title_match = track_name.contains(&norm_title);

    artist_match && title_match
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::time::Instant;

    use super::*;

    fn build_html_with_queries(queries: serde_json::Value) -> String {
        let next_data = serde_json::json!({
            "props": {
                "pageProps": {
                    "dehydratedState": {
                        "queries": queries
                    }
                }
            }
        });
        format!(
            r#"<html><head><script id="__NEXT_DATA__" type="application/json">{next_data}</script></head><body></body></html>"#
        )
    }

    fn build_html_with_tracks(tracks: serde_json::Value) -> String {
        build_html_with_queries(serde_json::json!([
            {
                "state": {
                    "data": {
                        "data": tracks
                    }
                }
            }
        ]))
    }

    #[test]
    fn test_parse_no_next_data() {
        let html = "<html><body>No data here</body></html>";
        let err = parse_beatport_html(html, "Burial", "Archangel")
            .expect_err("should fail on missing __NEXT_DATA__");
        assert!(
            matches!(&err, BeatportError::Parse(msg) if msg.contains("__NEXT_DATA__")),
            "error should be Parse mentioning __NEXT_DATA__, got: {err}"
        );
    }

    #[test]
    fn test_parse_returns_match() {
        let html = build_html_with_tracks(serde_json::json!([
            {
                "track_id": 12345,
                "track_name": "Archangel",
                "artists": [{"artist_name": "Burial"}],
                "bpm": 140,
                "key_name": "Am",
                "genre": [{"genre_name": "Bass / Club"}]
            }
        ]));

        let result = parse_beatport_html(&html, "Burial", "Archangel")
            .unwrap()
            .expect("expected a beatport match");

        assert_eq!(result.genre, "Bass / Club");
        assert_eq!(result.bpm, Some(140));
        assert_eq!(result.key, "Am");
        assert_eq!(result.track_name, "Archangel");
        assert_eq!(result.artists, vec!["Burial".to_string()]);
        assert_eq!(result.release_date, None);
        assert_eq!(result.label, None);
    }

    #[test]
    fn test_parse_extracts_label() {
        let html = build_html_with_tracks(serde_json::json!([
            {
                "track_id": 430213,
                "track_name": "Archangel",
                "artists": [{"artist_name": "Burial"}],
                "bpm": 135,
                "key_name": "C Minor",
                "genre": [{"genre_name": "Electronica"}],
                "label": {"label_id": 4553, "label_name": "Hyperdub"}
            }
        ]));

        let result = parse_beatport_html(&html, "Burial", "Archangel")
            .unwrap()
            .expect("expected a beatport match");

        assert_eq!(result.label, Some("Hyperdub".to_string()));
    }

    #[test]
    fn test_parse_extracts_publish_date() {
        let html = build_html_with_tracks(serde_json::json!([
            {
                "track_id": 1,
                "track_name": "Archangel",
                "artists": [{"artist_name": "Burial"}],
                "bpm": 140,
                "key_name": "Am",
                "genre": [{"genre_name": "Bass / Club"}],
                "publish_date": "2023-03-17T00:00:00",
                "release_date": "2023-01-01T00:00:00"
            }
        ]));

        let result = parse_beatport_html(&html, "Burial", "Archangel")
            .unwrap()
            .expect("expected a beatport match");

        assert_eq!(result.release_date, Some("2023-03-17T00:00:00".to_string()));
    }

    #[test]
    fn test_parse_falls_back_to_release_date() {
        let html = build_html_with_tracks(serde_json::json!([
            {
                "track_id": 1,
                "track_name": "Archangel",
                "artists": [{"artist_name": "Burial"}],
                "bpm": 140,
                "key_name": "Am",
                "genre": [{"genre_name": "Bass / Club"}],
                "release_date": "2010-12-06T00:00:00"
            }
        ]));

        let result = parse_beatport_html(&html, "Burial", "Archangel")
            .unwrap()
            .expect("expected a beatport match");

        assert_eq!(result.release_date, Some("2010-12-06T00:00:00".to_string()));
    }

    #[test]
    fn test_parse_returns_err_for_invalid_json() {
        let html = r#"<html><head><script id="__NEXT_DATA__" type="application/json">{invalid json}</script></head><body></body></html>"#;
        let err = parse_beatport_html(html, "Burial", "Archangel")
            .expect_err("should fail on malformed JSON");
        assert!(
            matches!(&err, BeatportError::Parse(msg) if msg.contains("malformed")),
            "error should be Parse mentioning malformed, got: {err}"
        );
    }

    #[test]
    fn test_parse_returns_none_when_no_match() {
        let html = build_html_with_tracks(serde_json::json!([
            {
                "track_id": 1,
                "track_name": "Different Track",
                "artists": [{"artist_name": "Different Artist"}],
                "bpm": 128,
                "key_name": "Bm",
                "genre": [{"genre_name": "Tech House"}]
            }
        ]));
        let result = parse_beatport_html(&html, "Burial", "Archangel").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_matches_case_insensitive_artist_and_title_substring() {
        let html = build_html_with_tracks(serde_json::json!([
            {
                "track_id": 2,
                "track_name": "Archangel (Extended Mix)",
                "artists": [{"artist_name": "BURIAL"}],
                "bpm": 138,
                "key_name": "Cm",
                "genre": [{"genre_name": "Leftfield Bass"}]
            }
        ]));

        let result = parse_beatport_html(&html, "burial", "Archangel")
            .unwrap()
            .expect("expected a beatport match");

        assert_eq!(result.track_name, "Archangel (Extended Mix)");
        assert_eq!(result.artists, vec!["BURIAL".to_string()]);
    }

    #[test]
    fn test_parse_no_match_when_search_title_is_longer_than_track_name() {
        // Matching is one-way: track_name must contain the search title, not vice versa.
        let html = build_html_with_tracks(serde_json::json!([
            {
                "track_id": 3,
                "track_name": "Archangel",
                "artists": [{"artist_name": "Burial"}],
                "bpm": 140,
                "key_name": "Am",
                "genre": [{"genre_name": "Bass / Club"}]
            }
        ]));
        let result = parse_beatport_html(&html, "Burial", "Archangel (Remastered)").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_finds_match_when_tracks_live_in_nonzero_query_index() {
        let html = build_html_with_queries(serde_json::json!([
            {
                "state": {
                    "data": {
                        "data": [
                            {
                                "track_id": 9,
                                "track_name": "Different Track",
                                "artists": [{"artist_name": "Different Artist"}],
                                "bpm": 125,
                                "key_name": "Fm",
                                "genre": [{"genre_name": "Tech House"}]
                            }
                        ]
                    }
                }
            },
            {
                "state": {
                    "data": {
                        "data": [
                            {
                                "track_id": 10,
                                "track_name": "Archangel",
                                "artists": [{"artist_name": "Burial"}],
                                "bpm": 140,
                                "key_name": "Am",
                                "genre": [{"genre_name": "Bass / Club"}]
                            }
                        ]
                    }
                }
            }
        ]));

        let result = parse_beatport_html(&html, "Burial", "Archangel")
            .unwrap()
            .expect("expected a beatport match from queries[1]");
        assert_eq!(result.track_name, "Archangel");
    }

    #[test]
    fn test_parse_does_not_match_when_track_name_is_empty() {
        let html = build_html_with_tracks(serde_json::json!([
            {
                "track_id": 4,
                "track_name": "",
                "artists": [{"artist_name": "Burial"}],
                "bpm": 140,
                "key_name": "Am",
                "genre": [{"genre_name": "Bass / Club"}]
            }
        ]));
        let result = parse_beatport_html(&html, "Burial", "Archangel").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_ignores_out_of_range_bpm() {
        let html = build_html_with_tracks(serde_json::json!([
            {
                "track_id": 5,
                "track_name": "Archangel",
                "artists": [{"artist_name": "Burial"}],
                "bpm": 4_294_967_295_i64,
                "key_name": "Am",
                "genre": [{"genre_name": "Bass / Club"}]
            }
        ]));

        let result = parse_beatport_html(&html, "Burial", "Archangel")
            .unwrap()
            .expect("expected a beatport match");
        assert_eq!(result.bpm, None);
    }

    #[test]
    fn test_is_track_match_rejects_empty_query_inputs() {
        let track = serde_json::json!({
            "track_name": "Archangel",
            "artists": [{"artist_name": "Burial"}]
        });

        assert!(!is_track_match(&track, "", "Archangel"));
        assert!(!is_track_match(&track, "Burial", ""));
        assert!(!is_track_match(&track, "   ", "Archangel"));
        assert!(!is_track_match(&track, "Burial", "   "));
    }

    #[test]
    fn test_parse_returns_first_match_when_multiple_tracks_match() {
        let html = build_html_with_tracks(serde_json::json!([
            {
                "track_id": 100,
                "track_name": "Archangel",
                "artists": [{"artist_name": "Burial"}],
                "bpm": 140,
                "key_name": "Am",
                "genre": [{"genre_name": "Bass / Club"}],
                "label": {"label_id": 1, "label_name": "Hyperdub"}
            },
            {
                "track_id": 200,
                "track_name": "Archangel (Extended Mix)",
                "artists": [{"artist_name": "Burial"}],
                "bpm": 138,
                "key_name": "Cm",
                "genre": [{"genre_name": "Leftfield Bass"}],
                "label": {"label_id": 2, "label_name": "Other Label"}
            },
            {
                "track_id": 300,
                "track_name": "Archangel (Remaster)",
                "artists": [{"artist_name": "Burial"}],
                "bpm": 140,
                "key_name": "Am",
                "genre": [{"genre_name": "Electronica"}],
                "label": {"label_id": 3, "label_name": "Third Label"}
            }
        ]));

        let result = parse_beatport_html(&html, "Burial", "Archangel")
            .unwrap()
            .expect("expected a beatport match");

        assert_eq!(result.track_name, "Archangel");
        assert_eq!(result.bpm, Some(140));
        assert_eq!(result.key, "Am");
        assert_eq!(result.genre, "Bass / Club");
        assert_eq!(result.label, Some("Hyperdub".to_string()));
        assert_eq!(result.artists, vec!["Burial".to_string()]);
        assert!(!result.fuzzy_match);
    }

    #[test]
    fn test_classify_http_status_404_is_no_match() {
        let result = classify_http_status(reqwest::StatusCode::NOT_FOUND, None);
        assert!(matches!(result, HttpStatusOutcome::NoMatch));
    }

    #[test]
    fn test_classify_http_status_429_includes_retry_after_and_retryable_context() {
        let result = classify_http_status(reqwest::StatusCode::TOO_MANY_REQUESTS, Some("30"));
        let HttpStatusOutcome::Error(err) = result else {
            panic!("429 should be treated as retryable error");
        };
        let msg = err.to_string();
        assert!(msg.contains("429 Too Many Requests"));
        assert!(msg.contains("transient/retryable"));
        assert!(msg.contains("Retry-After: 30"));
    }

    #[test]
    fn test_classify_http_status_5xx_is_retryable_error() {
        let result = classify_http_status(reqwest::StatusCode::BAD_GATEWAY, None);
        let HttpStatusOutcome::Error(err) = result else {
            panic!("5xx should be treated as retryable error");
        };
        let msg = err.to_string();
        assert!(msg.contains("502 Bad Gateway"));
        assert!(msg.contains("transient/retryable"));
    }

    #[test]
    fn test_classify_http_status_other_4xx_is_client_error() {
        let result = classify_http_status(reqwest::StatusCode::FORBIDDEN, None);
        let HttpStatusOutcome::Error(err) = result else {
            panic!("4xx (other than 404) should be treated as client error");
        };
        let msg = err.to_string();
        assert!(msg.contains("403 Forbidden"));
        assert!(msg.contains("client"));
    }

    #[tokio::test]
    async fn test_rate_limiter_enforces_minimum_spacing() {
        // SAFETY: test runs sequentially (no other threads reading this env var).
        unsafe { std::env::set_var("REKLAWDBOX_BEATPORT_MIN_INTERVAL_MS", "50") };

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

        *RATE_LIMITER.get().unwrap().lock().await = None;

        // SAFETY: cleaning up test-only env var.
        unsafe { std::env::remove_var("REKLAWDBOX_BEATPORT_MIN_INTERVAL_MS") };
    }
}
