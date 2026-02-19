use reqwest::Client;
use serde::{Deserialize, Serialize};

const BEATPORT_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
    (KHTML, like Gecko) Chrome/91.0.4472.124 Safari/537.36";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeatportResult {
    pub genre: String,
    pub bpm: Option<i32>,
    pub key: String,
    pub track_name: String,
    pub artists: Vec<String>,
}

/// Look up a track on Beatport. Returns None if no match found.
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

    if !resp.status().is_success() {
        return Ok(None);
    }

    let html = resp
        .text()
        .await
        .map_err(|e| format!("read body failed: {e}"))?;

    parse_beatport_html(&html, artist, title)
}

/// Parse Beatport HTML to extract track data from __NEXT_DATA__ JSON.
fn parse_beatport_html(
    html: &str,
    artist: &str,
    title: &str,
) -> Result<Option<BeatportResult>, String> {
    // Find __NEXT_DATA__ script tag
    let marker = "__NEXT_DATA__\" type=\"application/json\">";
    let start = match html.find(marker) {
        Some(pos) => pos + marker.len(),
        None => return Ok(None),
    };
    let end = match html[start..].find("</script>") {
        Some(pos) => start + pos,
        None => return Ok(None),
    };

    let json_str = &html[start..end];
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

    let norm_artist = artist.to_lowercase();
    let norm_title = title.to_lowercase();

    for track in tracks {
        // Check artist match
        let empty = Vec::new();
        let artists = track
            .get("artists")
            .and_then(|v| v.as_array())
            .unwrap_or(&empty)
            .iter()
            .filter_map(|a| a.get("artist_name").and_then(|n| n.as_str()))
            .collect::<Vec<_>>();

        let artist_match = artists.iter().any(|a| a.to_lowercase() == norm_artist);

        // Check title match (bidirectional substring)
        let track_name = track
            .get("track_name")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let track_name_lower = track_name.to_lowercase();
        let title_match =
            track_name_lower.contains(&norm_title) || norm_title.contains(&track_name_lower);

        if artist_match && title_match {
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
                artists: artists.into_iter().map(|s| s.to_string()).collect(),
            }));
        }
    }

    Ok(None)
}

/// Percent-encode a string for URL query parameters.
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

    #[test]
    fn test_parse_no_next_data() {
        let html = "<html><body>No data here</body></html>";
        let result = parse_beatport_html(html, "Burial", "Archangel").unwrap();
        assert!(result.is_none());
    }
}
