use reqwest::Client;
use serde::{Deserialize, Serialize};

const BEATPORT_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
    (KHTML, like Gecko) Chrome/91.0.4472.124 Safari/537.36";

enum HttpStatusHandling {
    NoMatch,
    Error(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeatportResult {
    pub genre: String,
    pub bpm: Option<i32>,
    pub key: String,
    pub track_name: String,
    pub artists: Vec<String>,
}

pub async fn lookup(
    client: &Client,
    artist: &str,
    title: &str,
) -> Result<Option<BeatportResult>, String> {
    // Rate limit
    tokio::time::sleep(std::time::Duration::from_millis(2000)).await;

    let query = format!("{artist} {title}");
    let url = format!(
        "https://www.beatport.com/search/tracks?q={}",
        urlencoding(&query)
    );

    let resp = client
        .get(&url)
        .header("User-Agent", BEATPORT_UA)
        .header(
            "Accept",
            "text/html,application/xhtml+xml,application/xml;q=0.9,image/webp,*/*;q=0.8",
        )
        .header("Accept-Language", "en-US,en;q=0.5")
        .header("Cache-Control", "no-cache")
        .header("Pragma", "no-cache")
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        let retry_after = resp
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        return match classify_http_status(status, retry_after.as_deref()) {
            HttpStatusHandling::NoMatch => Ok(None),
            HttpStatusHandling::Error(msg) => Err(msg),
        };
    }

    let html = resp
        .text()
        .await
        .map_err(|e| format!("read body failed: {e}"))?;

    parse_beatport_html(&html, artist, title)
}

fn classify_http_status(
    status: reqwest::StatusCode,
    retry_after: Option<&str>,
) -> HttpStatusHandling {
    if status == reqwest::StatusCode::NOT_FOUND {
        return HttpStatusHandling::NoMatch;
    }

    if status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
        return HttpStatusHandling::Error(http_status_error(
            status,
            retry_after,
            "transient/retryable",
        ));
    }

    if status.is_client_error() {
        return HttpStatusHandling::Error(http_status_error(status, retry_after, "client"));
    }

    HttpStatusHandling::Error(http_status_error(status, retry_after, "unexpected"))
}

fn http_status_error(status: reqwest::StatusCode, retry_after: Option<&str>, kind: &str) -> String {
    match retry_after {
        Some(delay) if !delay.is_empty() => {
            format!("Beatport {kind} HTTP {status} (Retry-After: {delay})")
        }
        _ => format!("Beatport {kind} HTTP {status}"),
    }
}

/// Parse Beatport HTML to extract track data from __NEXT_DATA__ JSON.
fn parse_beatport_html(
    html: &str,
    artist: &str,
    title: &str,
) -> Result<Option<BeatportResult>, String> {
    let json_str = match extract_next_data_json(html) {
        Some(v) => v,
        None => return Ok(None),
    };
    let next_data: serde_json::Value = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(_) => return Ok(None),
    };

    // Navigate: props.pageProps.dehydratedState.queries[0].state.data.data
    let tracks = next_data.pointer("/props/pageProps/dehydratedState/queries/0/state/data/data");

    let tracks = match tracks.and_then(|v| v.as_array()) {
        Some(arr) => arr,
        None => return Ok(None),
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

            let bpm = track.get("bpm").and_then(|v| v.as_i64()).map(|v| v as i32);

            let key = track
                .get("key_name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            return Ok(Some(BeatportResult {
                genre,
                bpm,
                key,
                track_name: track_name.to_string(),
                artists,
            }));
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
    let norm_artist = artist.to_lowercase();
    let norm_title = title.to_lowercase();

    let artist_match = track
        .get("artists")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|a| a.get("artist_name").and_then(|n| n.as_str()))
                .any(|name| name.to_lowercase() == norm_artist)
        })
        .unwrap_or(false);

    let track_name = track
        .get("track_name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_lowercase();
    let title_match = track_name.contains(&norm_title) || norm_title.contains(&track_name);

    artist_match && title_match
}

fn urlencoding(s: &str) -> String {
    use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
    const SET: &AsciiSet = &NON_ALPHANUMERIC
        .remove(b'-')
        .remove(b'_')
        .remove(b'.')
        .remove(b'~');
    utf8_percent_encode(s, SET).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_html_with_tracks(tracks: serde_json::Value) -> String {
        let next_data = serde_json::json!({
            "props": {
                "pageProps": {
                    "dehydratedState": {
                        "queries": [
                            {
                                "state": {
                                    "data": {
                                        "data": tracks
                                    }
                                }
                            }
                        ]
                    }
                }
            }
        });
        format!(
            r#"<html><head><script id="__NEXT_DATA__" type="application/json">{}</script></head><body></body></html>"#,
            next_data
        )
    }

    #[test]
    fn test_parse_no_next_data() {
        let html = "<html><body>No data here</body></html>";
        let result = parse_beatport_html(html, "Burial", "Archangel").unwrap();
        assert!(result.is_none());
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
    }

    #[test]
    fn test_parse_returns_none_for_invalid_json() {
        let html = r#"<html><head><script id="__NEXT_DATA__" type="application/json">{invalid json}</script></head><body></body></html>"#;
        let result = parse_beatport_html(html, "Burial", "Archangel").unwrap();
        assert!(result.is_none());
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
    fn test_parse_matches_when_search_title_contains_track_title() {
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
        let result = parse_beatport_html(&html, "Burial", "Archangel (Remastered)")
            .unwrap()
            .expect("expected a beatport match");
        assert_eq!(result.track_name, "Archangel");
    }

    #[test]
    fn test_classify_http_status_404_is_no_match() {
        let result = classify_http_status(reqwest::StatusCode::NOT_FOUND, None);
        assert!(matches!(result, HttpStatusHandling::NoMatch));
    }

    #[test]
    fn test_classify_http_status_429_includes_retry_after_and_retryable_context() {
        let result = classify_http_status(reqwest::StatusCode::TOO_MANY_REQUESTS, Some("30"));
        let HttpStatusHandling::Error(msg) = result else {
            panic!("429 should be treated as retryable error");
        };
        assert!(msg.contains("429 Too Many Requests"));
        assert!(msg.contains("transient/retryable"));
        assert!(msg.contains("Retry-After: 30"));
    }

    #[test]
    fn test_classify_http_status_5xx_is_retryable_error() {
        let result = classify_http_status(reqwest::StatusCode::BAD_GATEWAY, None);
        let HttpStatusHandling::Error(msg) = result else {
            panic!("5xx should be treated as retryable error");
        };
        assert!(msg.contains("502 Bad Gateway"));
        assert!(msg.contains("transient/retryable"));
    }

    #[test]
    fn test_classify_http_status_other_4xx_is_client_error() {
        let result = classify_http_status(reqwest::StatusCode::FORBIDDEN, None);
        let HttpStatusHandling::Error(msg) = result else {
            panic!("4xx (other than 404) should be treated as client error");
        };
        assert!(msg.contains("403 Forbidden"));
        assert!(msg.contains("client"));
    }
}
