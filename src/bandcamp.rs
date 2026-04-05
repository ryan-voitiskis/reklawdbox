use std::sync::{LazyLock, OnceLock};

use regex::Regex;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::normalize::urlencoding;

crate::rate_limit::define_rate_limiter!("REKLAWDBOX_BANDCAMP_MIN_INTERVAL_MS", 1500);

static URL_RE: OnceLock<Regex> = OnceLock::new();

fn url_re() -> &'static Regex {
    URL_RE.get_or_init(|| {
        Regex::new(r#"<a\b[^>]*\bhref="(https?://[^"]+\.bandcamp\.com/track/[^"?]+)"#).unwrap()
    })
}

const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 \
    (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

#[derive(Debug, thiserror::Error)]
pub enum BandcampError {
    #[error("Bandcamp HTTP {status}")]
    Http { status: reqwest::StatusCode },
    #[error("{0}")]
    Parse(String),
    #[error("{0}")]
    Request(#[from] reqwest::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BandcampResult {
    pub track_title: String,
    pub artist_name: String,
    pub release_date: Option<String>,
    pub label: Option<String>,
    pub tags: Vec<String>,
    pub album: Option<String>,
    pub bandcamp_url: String,
    pub score: i32,
}

pub async fn lookup(
    client: &Client,
    artist: &str,
    title: &str,
) -> Result<Option<BandcampResult>, BandcampError> {
    wait_for_rate_limit().await;

    let search_query = build_search_query(artist, title);
    let url = format!(
        "https://bandcamp.com/search?q={}&item_type=t",
        urlencoding(&search_query)
    );

    let resp = client
        .get(&url)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "text/html")
        .header("Accept-Language", "en-US,en;q=0.5")
        .send()
        .await?;

    let status = resp.status();
    if !status.is_success() {
        return Err(BandcampError::Http { status });
    }

    let html = resp.text().await?;
    let candidates = parse_search_results(&html);

    if candidates.is_empty() {
        return Ok(None);
    }

    let mut best: Option<(SearchResult, i32)> = None;
    for candidate in candidates {
        let score = score_match(artist, title, &candidate.artist, &candidate.title);
        if score >= 65 && best.as_ref().is_none_or(|(_, s)| score > *s) {
            best = Some((candidate, score));
        }
    }

    let Some((matched, score)) = best else {
        return Ok(None);
    };

    wait_for_rate_limit().await;

    let detail = match fetch_detail(client, &matched.url).await {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!(url = %matched.url, "Bandcamp detail fetch failed, returning search-only data: {e}");
            DetailResult {
                artist: None,
                label: None,
                date_published: None,
                tags: None,
            }
        }
    };

    let release_date = detail
        .date_published
        .as_deref()
        .or(matched.date.as_deref())
        .and_then(normalize_date_to_iso);

    Ok(Some(BandcampResult {
        track_title: matched.title,
        artist_name: detail.artist.unwrap_or(matched.artist),
        release_date,
        label: detail.label,
        tags: detail.tags.unwrap_or_default(),
        album: matched.album,
        bandcamp_url: matched.url,
        score,
    }))
}

/// Bandcamp uses "DD Mon YYYY ..." (JSON-LD) and "Month DD, YYYY" (search).
/// Normalizes both to "YYYY-MM-DD".
fn normalize_date_to_iso(s: &str) -> Option<String> {
    let s = s.trim();

    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() >= 3
        && let (Some(day), Some(month_num), Some(year)) = (
            parts[0]
                .parse::<u32>()
                .ok()
                .filter(|d| (1..=31).contains(d)),
            month_abbrev_to_num(parts[1]),
            parts[2]
                .parse::<i32>()
                .ok()
                .filter(|y| (1900..=2099).contains(y)),
        )
    {
        return Some(format!("{year:04}-{month_num:02}-{day:02}"));
    }

    if parts.len() >= 3 {
        let day_str = parts[1].trim_end_matches(',');
        if let (Some(month_num), Some(day), Some(year)) = (
            month_name_to_num(parts[0]),
            day_str.parse::<u32>().ok().filter(|d| (1..=31).contains(d)),
            parts[2]
                .parse::<i32>()
                .ok()
                .filter(|y| (1900..=2099).contains(y)),
        ) {
            return Some(format!("{year:04}-{month_num:02}-{day:02}"));
        }
    }

    if s.len() >= 4 && s[..4].chars().all(|c| c.is_ascii_digit()) {
        return Some(s.to_string());
    }

    None
}

fn month_abbrev_to_num(s: &str) -> Option<u32> {
    match s.to_lowercase().as_str() {
        "jan" => Some(1),
        "feb" => Some(2),
        "mar" => Some(3),
        "apr" => Some(4),
        "may" => Some(5),
        "jun" => Some(6),
        "jul" => Some(7),
        "aug" => Some(8),
        "sep" => Some(9),
        "oct" => Some(10),
        "nov" => Some(11),
        "dec" => Some(12),
        _ => None,
    }
}

fn month_name_to_num(s: &str) -> Option<u32> {
    match s.to_lowercase().as_str() {
        "january" => Some(1),
        "february" => Some(2),
        "march" => Some(3),
        "april" => Some(4),
        "may" => Some(5),
        "june" => Some(6),
        "july" => Some(7),
        "august" => Some(8),
        "september" => Some(9),
        "october" => Some(10),
        "november" => Some(11),
        "december" => Some(12),
        _ => month_abbrev_to_num(s),
    }
}

fn build_search_query(artist: &str, title: &str) -> String {
    let clean_title = strip_title_noise(title);
    format!("{artist} {clean_title}")
}

fn strip_title_noise(title: &str) -> String {
    let mut s = title.to_string();

    for ext in [".wav", ".mp3", ".aif", ".aiff", ".flac"] {
        if s.to_lowercase().ends_with(ext) {
            s.truncate(s.len() - ext.len());
        }
    }

    static ORIGINAL_RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)\s*\(Original(?:\s+(?:Mix|Version))?\)$")
            .expect("ORIGINAL_RE must compile")
    });
    s = ORIGINAL_RE.replace(&s, "").into_owned();

    s.trim().to_string()
}

#[derive(Debug, Clone)]
struct SearchResult {
    url: String,
    title: String,
    artist: String,
    album: Option<String>,
    date: Option<String>,
}

fn parse_search_results(html: &str) -> Vec<SearchResult> {
    let blocks: Vec<&str> = html.split("class=\"searchresult").skip(1).collect();

    blocks
        .iter()
        .filter_map(|block| {
            let url = url_re().captures(block)?.get(1)?.as_str().to_string();
            let title = extract_heading_text(block)?;
            let (artist, album) = extract_artist_album(block);
            let artist = artist?;
            let date = extract_released_date(block);

            Some(SearchResult {
                url,
                title,
                artist,
                album,
                date,
            })
        })
        .collect()
}

fn extract_heading_text(block: &str) -> Option<String> {
    let heading_start = block.find("class=\"heading\"")?;
    let section = &block[heading_start..];

    let a_start = section.find('>')? + 1;
    let link_start = section[a_start..].find('>')? + a_start + 1;
    let link_end = section[link_start..].find("</a>")? + link_start;

    let text = section[link_start..link_end].trim();
    if text.is_empty() {
        return None;
    }
    Some(strip_html_tags(text))
}

/// Parses "by Artist" or "from Album by Artist" from the subhead.
fn extract_artist_album(block: &str) -> (Option<String>, Option<String>) {
    let subhead_start = match block.find("class=\"subhead\"") {
        Some(pos) => pos,
        None => return (None, None),
    };
    let section = &block[subhead_start..];
    let content_start = match section.find('>') {
        Some(pos) => pos + 1,
        None => return (None, None),
    };
    let content_end = match section[content_start..].find("</div>") {
        Some(pos) => pos + content_start,
        None => section.len(),
    };
    let text = strip_html_tags(section[content_start..content_end].trim());
    let text = text.trim();

    if let Some(from_pos) = text.find("from ") {
        let after_from = &text[from_pos + 5..];
        if let Some(by_pos) = after_from.rfind(" by ") {
            let album = after_from[..by_pos].trim().to_string();
            let artist = after_from[by_pos + 4..].trim().to_string();
            if !artist.is_empty() {
                let album = if album.is_empty() { None } else { Some(album) };
                return (Some(artist), album);
            }
        }
    }

    if let Some(by_pos) = text.find("by ") {
        let artist = text[by_pos + 3..].trim().to_string();
        if !artist.is_empty() {
            return (Some(artist), None);
        }
    }

    (None, None)
}

fn extract_released_date(block: &str) -> Option<String> {
    let pos = block.find("released ")?;
    let after = &block[pos + 9..];

    let end = after
        .find('<')
        .unwrap_or(after.len())
        .min(after.find('\n').unwrap_or(after.len()));

    let date = after[..end].trim().to_string();
    if date.is_empty() { None } else { Some(date) }
}

fn strip_html_tags(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        if c == '<' {
            in_tag = true;
        } else if c == '>' {
            in_tag = false;
        } else if !in_tag {
            result.push(c);
        }
    }
    result.trim().to_string()
}

#[derive(Debug)]
struct DetailResult {
    artist: Option<String>,
    label: Option<String>,
    date_published: Option<String>,
    tags: Option<Vec<String>>,
}

async fn fetch_detail(client: &Client, url: &str) -> Result<DetailResult, BandcampError> {
    let resp = client
        .get(url)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "text/html")
        .header("Accept-Language", "en-US,en;q=0.5")
        .send()
        .await?;

    let status = resp.status();
    if !status.is_success() {
        return Err(BandcampError::Http { status });
    }

    let html = resp.text().await?;
    parse_detail_json_ld(&html)
}

fn parse_detail_json_ld(html: &str) -> Result<DetailResult, BandcampError> {
    let json_str = match extract_json_ld(html) {
        Some(s) => s,
        None => {
            tracing::warn!(
                "Bandcamp detail page has no JSON-LD block — page structure may have changed"
            );
            return Ok(DetailResult {
                artist: None,
                label: None,
                date_published: None,
                tags: None,
            });
        }
    };

    let data: serde_json::Value = serde_json::from_str(json_str)
        .map_err(|e| BandcampError::Parse(format!("Bandcamp JSON-LD malformed: {e}")))?;

    let artist = data
        .get("byArtist")
        .and_then(|v| v.get("name"))
        .and_then(|v| v.as_str())
        .map(str::to_string);

    let label = data
        .get("publisher")
        .and_then(|v| v.get("name"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
        // publisher == artist means self-released; no useful label signal
        .filter(|l| artist.as_deref() != Some(l.as_str()));

    let date_published = data
        .get("datePublished")
        .and_then(|v| v.as_str())
        .map(str::to_string);

    // keywords may be a comma-separated string or a JSON array
    let tags = data.get("keywords").and_then(|v| {
        if let Some(arr) = v.as_array() {
            let tags: Vec<String> = arr
                .iter()
                .filter_map(|v| v.as_str())
                .map(str::to_string)
                .collect();
            if tags.is_empty() { None } else { Some(tags) }
        } else if let Some(s) = v.as_str() {
            let tags: Vec<String> = s
                .split(',')
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect();
            if tags.is_empty() { None } else { Some(tags) }
        } else {
            None
        }
    });

    Ok(DetailResult {
        artist,
        label,
        date_published,
        tags,
    })
}

fn extract_json_ld(html: &str) -> Option<&str> {
    let marker = "application/ld+json";
    let marker_pos = html.find(marker)?;
    let script_start = html[..marker_pos].rfind("<script")?;
    let open_tag_end = html[script_start..].find('>')? + script_start + 1;
    let script_end = html[open_tag_end..].find("</script>")? + open_tag_end;

    Some(html[open_tag_end..script_end].trim())
}

fn score_match(
    query_artist: &str,
    query_title: &str,
    result_artist: &str,
    result_title: &str,
) -> i32 {
    let norm_qa = normalize_for_comparison(query_artist);
    let norm_ra = normalize_for_comparison(result_artist);

    let norm_qt = normalize_for_comparison(&strip_paren_suffix(query_title));
    let norm_rt = normalize_for_comparison(&strip_paren_suffix(result_title));

    if norm_qa.is_empty() || norm_qt.is_empty() || norm_ra.is_empty() || norm_rt.is_empty() {
        return 0;
    }

    let artist_score = normalized_levenshtein(&norm_qa, &norm_ra);

    let title_lev = normalized_levenshtein(&norm_qt, &norm_rt);
    let title_prefix = if norm_rt.starts_with(&norm_qt) || norm_qt.starts_with(&norm_rt) {
        let shorter = norm_qt.len().min(norm_rt.len()) as f64;
        let longer = norm_qt.len().max(norm_rt.len()) as f64;
        (shorter / longer).max(0.85)
    } else {
        0.0
    };
    let title_score = title_lev.max(title_prefix);

    ((artist_score * 0.4 + title_score * 0.6) * 100.0) as i32
}

fn strip_paren_suffix(s: &str) -> String {
    let trimmed = s.trim();
    if let Some(pos) = trimmed.rfind('(') {
        let before = trimmed[..pos].trim();
        if !before.is_empty() {
            return before.to_string();
        }
    }
    trimmed.to_string()
}

fn normalize_for_comparison(s: &str) -> String {
    let lower = s.to_lowercase();

    let s = if let Some(pos) = lower.find(" feat.") {
        &lower[..pos]
    } else if let Some(pos) = lower.find(" feat ") {
        &lower[..pos]
    } else if let Some(pos) = lower.find(" ft.") {
        &lower[..pos]
    } else if let Some(pos) = lower.find(" ft ") {
        &lower[..pos]
    } else {
        &lower
    };

    s.chars()
        .filter(|c| c.is_alphanumeric() || *c == ' ')
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalized_levenshtein(a: &str, b: &str) -> f64 {
    if a == b {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }

    let len_a = a.chars().count();
    let len_b = b.chars().count();
    let max_len = len_a.max(len_b);

    let b_chars: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=len_b).collect();
    let mut curr = vec![0usize; len_b + 1];

    for (i, ca) in a.chars().enumerate() {
        curr[0] = i + 1;
        for (j, &cb) in b_chars.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            curr[j + 1] = (prev[j + 1] + 1).min(curr[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }

    let distance = prev[len_b];
    1.0 - (distance as f64 / max_len as f64)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::time::Instant;

    use super::*;

    #[test]
    fn strip_file_extensions() {
        assert_eq!(strip_title_noise("Paradisea.wav"), "Paradisea");
        assert_eq!(strip_title_noise("Track.mp3"), "Track");
        assert_eq!(strip_title_noise("Song.flac"), "Song");
        assert_eq!(strip_title_noise("Mix.aif"), "Mix");
    }

    #[test]
    fn strip_original_mix() {
        assert_eq!(
            strip_title_noise("Energy Soul (Original Mix)"),
            "Energy Soul"
        );
    }

    #[test]
    fn strip_original_bare() {
        assert_eq!(strip_title_noise("Energy Soul (Original)"), "Energy Soul");
    }

    #[test]
    fn strip_original_version() {
        assert_eq!(
            strip_title_noise("Energy Soul (Original Version)"),
            "Energy Soul"
        );
    }

    #[test]
    fn no_strip_original_club_mix() {
        assert_eq!(
            strip_title_noise("Track (Original Club Mix)"),
            "Track (Original Club Mix)"
        );
    }

    #[test]
    fn no_strip_when_not_suffix() {
        assert_eq!(strip_title_noise("Archangel (Remix)"), "Archangel (Remix)");
    }

    #[test]
    fn normalize_strips_feat() {
        assert_eq!(
            normalize_for_comparison("Nina Kraviz feat. King Aus"),
            "nina kraviz"
        );
        assert_eq!(normalize_for_comparison("Le Motel ft. Flowdan"), "le motel");
    }

    #[test]
    fn normalize_keeps_alphanumeric() {
        assert_eq!(normalize_for_comparison("Mr. G"), "mr g");
        assert_eq!(normalize_for_comparison("Cos-Ber-Zam"), "cosberzam");
    }

    #[test]
    fn levenshtein_identical() {
        assert_eq!(normalized_levenshtein("hello", "hello"), 1.0);
    }

    #[test]
    fn levenshtein_empty() {
        assert_eq!(normalized_levenshtein("", "hello"), 0.0);
        assert_eq!(normalized_levenshtein("hello", ""), 0.0);
    }

    #[test]
    fn levenshtein_similar() {
        let score = normalized_levenshtein("canned", "caned");
        assert!(score > 0.8, "expected > 0.8, got {score}");
    }

    #[test]
    fn levenshtein_different() {
        let score = normalized_levenshtein("hello", "world");
        assert!(score < 0.5, "expected < 0.5, got {score}");
    }

    #[test]
    fn score_exact_match() {
        assert_eq!(
            score_match("Fred P", "Energy Soul", "Fred P", "Energy Soul"),
            100
        );
    }

    #[test]
    fn score_similar_title() {
        let score = score_match("Will Hofbauer", "Canned", "Will Hofbauer", "Caned");
        assert!(score >= 70, "expected >= 70, got {score}");
    }

    #[test]
    fn score_with_feat() {
        let score = score_match(
            "Nina Kraviz feat. King Aus On The Mic",
            "Aus",
            "Nina Kraviz",
            "Aus (DJ Qu remix)",
        );
        assert!(score >= 65, "expected >= 65, got {score}");
    }

    #[test]
    fn score_total_mismatch() {
        let score = score_match("Fred P", "Energy Soul", "Burial", "Archangel");
        assert!(score < 40, "expected < 40, got {score}");
    }

    #[test]
    fn strips_tags() {
        assert_eq!(strip_html_tags("<b>hello</b>"), "hello");
        assert_eq!(strip_html_tags("no tags"), "no tags");
    }

    #[test]
    fn extract_by_artist() {
        let block = r#"class="subhead">by Fred P</div>"#;
        let (artist, album) = extract_artist_album(block);
        assert_eq!(artist.as_deref(), Some("Fred P"));
        assert!(album.is_none());
    }

    #[test]
    fn extract_from_album_by_artist() {
        let block = r#"class="subhead">
            from Energy Soul
             by Fred P</div>"#;
        let (artist, album) = extract_artist_album(block);
        assert_eq!(artist.as_deref(), Some("Fred P"));
        assert_eq!(album.as_deref(), Some("Energy Soul"));
    }

    #[test]
    fn extract_date() {
        let block = "released October 28, 2016\n";
        assert_eq!(
            extract_released_date(block).as_deref(),
            Some("October 28, 2016")
        );
    }

    #[test]
    fn extract_date_with_html() {
        let block = "released January 1, 2015<br/>";
        assert_eq!(
            extract_released_date(block).as_deref(),
            Some("January 1, 2015")
        );
    }

    #[test]
    fn extract_json_ld_basic() {
        let html = r#"<html><head>
            <script type="application/ld+json">{"@type":"MusicRecording","byArtist":{"name":"Fred P"}}</script>
            </head></html>"#;
        let json = extract_json_ld(html).unwrap();
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        assert_eq!(
            v.get("byArtist")
                .unwrap()
                .get("name")
                .unwrap()
                .as_str()
                .unwrap(),
            "Fred P"
        );
    }

    #[test]
    fn extract_json_ld_missing() {
        let html = "<html><body>no json-ld here</body></html>";
        assert!(extract_json_ld(html).is_none());
    }

    #[test]
    fn parse_detail_extracts_all_fields() {
        let html = r#"<html><head>
            <script type="application/ld+json">{
                "@type": "MusicRecording",
                "byArtist": {"name": "Fred P"},
                "publisher": {"name": "Ibadan Records"},
                "datePublished": "28 Oct 2016",
                "keywords": ["deep house", "techno"]
            }</script>
            </head></html>"#;
        let detail = parse_detail_json_ld(html).unwrap();
        assert_eq!(detail.artist.as_deref(), Some("Fred P"));
        assert_eq!(detail.label.as_deref(), Some("Ibadan Records"));
        assert_eq!(detail.date_published.as_deref(), Some("28 Oct 2016"));
        assert_eq!(detail.tags.unwrap(), vec!["deep house", "techno"]);
    }

    #[test]
    fn normalize_json_ld_date() {
        assert_eq!(
            normalize_date_to_iso("28 Oct 2016 00:00:00 GMT").as_deref(),
            Some("2016-10-28")
        );
        assert_eq!(
            normalize_date_to_iso("01 Jan 2015 00:00:00 GMT").as_deref(),
            Some("2015-01-01")
        );
    }

    #[test]
    fn normalize_search_page_date() {
        assert_eq!(
            normalize_date_to_iso("October 28, 2016").as_deref(),
            Some("2016-10-28")
        );
        assert_eq!(
            normalize_date_to_iso("January 1, 2015").as_deref(),
            Some("2015-01-01")
        );
        assert_eq!(
            normalize_date_to_iso("August 28, 2024").as_deref(),
            Some("2024-08-28")
        );
    }

    #[test]
    fn normalize_already_iso() {
        assert_eq!(
            normalize_date_to_iso("2016-10-28").as_deref(),
            Some("2016-10-28")
        );
        assert_eq!(normalize_date_to_iso("2019").as_deref(), Some("2019"));
    }

    #[test]
    fn normalize_invalid_date() {
        assert!(normalize_date_to_iso("no date here").is_none());
        assert!(normalize_date_to_iso("").is_none());
    }

    #[test]
    fn parse_detail_keywords_as_string() {
        let html = r#"<html><head>
            <script type="application/ld+json">{
                "byArtist": {"name": "Fred P"},
                "publisher": {"name": "Ibadan Records"},
                "keywords": "deep house, techno, Berlin"
            }</script>
            </head></html>"#;
        let detail = parse_detail_json_ld(html).unwrap();
        assert_eq!(detail.tags.unwrap(), vec!["deep house", "techno", "Berlin"]);
    }

    #[test]
    fn parse_detail_filters_self_released() {
        let html = r#"<html><head>
            <script type="application/ld+json">{
                "byArtist": {"name": "Solo Artist"},
                "publisher": {"name": "Solo Artist"}
            }</script>
            </head></html>"#;
        let detail = parse_detail_json_ld(html).unwrap();
        assert!(
            detail.label.is_none(),
            "self-released should be filtered out"
        );
    }

    #[test]
    fn parse_detail_no_json_ld_returns_empty() {
        let html = "<html><body>plain page</body></html>";
        let detail = parse_detail_json_ld(html).unwrap();
        assert!(detail.artist.is_none());
        assert!(detail.label.is_none());
    }

    #[test]
    fn parse_search_results_extracts_track() {
        let html = r#"
            <div class="searchresult data-search='{"type":"t","id":1}'>
                <div class="result-info">
                    <div class="heading">
                        <a href="https://fred-p.bandcamp.com/track/energy-soul">Energy Soul</a>
                    </div>
                    <div class="subhead">by Fred P</div>
                    <div class="released">released October 28, 2016</div>
                </div>
            </div>
        "#;
        let results = parse_search_results(html);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Energy Soul");
        assert_eq!(results[0].artist, "Fred P");
        assert_eq!(
            results[0].url,
            "https://fred-p.bandcamp.com/track/energy-soul"
        );
        assert_eq!(results[0].date.as_deref(), Some("October 28, 2016"));
    }

    #[test]
    fn parse_search_results_with_album() {
        let html = r#"
            <div class="searchresult data-search='{"type":"t","id":2}'>
                <div class="result-info">
                    <div class="heading">
                        <a href="https://artist.bandcamp.com/track/my-track">My Track</a>
                    </div>
                    <div class="subhead">from My Album by Some Artist</div>
                    <div class="released">released January 1, 2020</div>
                </div>
            </div>
        "#;
        let results = parse_search_results(html);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].artist, "Some Artist");
        assert_eq!(results[0].album.as_deref(), Some("My Album"));
    }

    #[test]
    fn parse_search_results_ignores_album_items() {
        // Non-track URLs (album pages) should not match the track URL regex
        let html = r#"
            <div class="searchresult">
                <div class="result-info">
                    <div class="heading">
                        <a href="https://artist.bandcamp.com/album/my-album">My Album</a>
                    </div>
                    <div class="subhead">by Some Artist</div>
                </div>
            </div>
        "#;
        let results = parse_search_results(html);
        assert!(
            results.is_empty(),
            "album URLs should not match track regex"
        );
    }

    #[tokio::test]
    async fn rate_limiter_enforces_minimum_spacing() {
        // SAFETY: test runs sequentially (no other threads reading this env var).
        unsafe { std::env::set_var("REKLAWDBOX_BANDCAMP_MIN_INTERVAL_MS", "50") };

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
        unsafe { std::env::remove_var("REKLAWDBOX_BANDCAMP_MIN_INTERVAL_MS") };
    }
}
