use std::fmt::Write;
use std::fs;
use std::path::Path;

use crate::types::Track;

/// Escape special characters for XML attribute values.
pub fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

/// Convert a file system path to a Rekordbox Location URI.
/// e.g. `/Users/vz/Music/file name.flac` → `file://localhost/Users/vz/Music/file%20name.flac`
pub fn path_to_location(file_path: &str) -> String {
    use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};

    // Encode everything except unreserved chars and path separators
    const ENCODE_SET: &AsciiSet = &NON_ALPHANUMERIC
        .remove(b'-')
        .remove(b'_')
        .remove(b'.')
        .remove(b'~')
        .remove(b'/');

    let encoded = utf8_percent_encode(file_path, ENCODE_SET).to_string();
    format!("file://localhost{encoded}")
}

/// Map file_type integer to Kind string for XML.
fn file_type_to_kind(file_type: i32) -> &'static str {
    match file_type {
        1 => "MP3 File",
        4 => "M4A File",
        5 => "FLAC File",
        11 => "WAV File",
        12 => "AIFF File",
        _ => "Audio File",
    }
}

/// Generate a single TRACK XML element.
fn write_track(out: &mut String, track: &Track, track_id: usize) {
    let rating = crate::types::stars_to_rating(track.rating);
    let location = path_to_location(&track.file_path);
    let kind = file_type_to_kind(track.file_type);

    write!(
        out,
        "      <TRACK TrackID=\"{track_id}\" \
         Name=\"{name}\" \
         Artist=\"{artist}\" \
         Composer=\"\" \
         Album=\"{album}\" \
         Grouping=\"\" \
         Genre=\"{genre}\" \
         Kind=\"{kind}\" \
         Size=\"0\" \
         TotalTime=\"{total_time}\" \
         DiscNumber=\"0\" \
         TrackNumber=\"0\" \
         Year=\"{year}\" \
         AverageBpm=\"{bpm:.2}\" \
         DateAdded=\"{date_added}\" \
         BitRate=\"{bit_rate}\" \
         SampleRate=\"{sample_rate}\" \
         Comments=\"{comments}\" \
         PlayCount=\"{play_count}\" \
         Rating=\"{rating}\" \
         Location=\"{location}\" \
         Remixer=\"{remixer}\" \
         Tonality=\"{key}\" \
         Label=\"{label}\" \
         Mix=\"\"",
        name = xml_escape(&track.title),
        artist = xml_escape(&track.artist),
        album = xml_escape(&track.album),
        genre = xml_escape(&track.genre),
        total_time = track.length,
        year = track.year,
        bpm = track.bpm,
        date_added = xml_escape(&track.date_added),
        bit_rate = track.bit_rate,
        sample_rate = track.sample_rate,
        comments = xml_escape(&track.comments),
        play_count = track.play_count,
        remixer = xml_escape(&track.remixer),
        key = xml_escape(&track.key),
        label = xml_escape(&track.label),
    )
    .unwrap();

    // Add Colour attribute if color is set (hex format: 0xRRGGBB)
    if track.color_code > 0 {
        write!(out, " Colour=\"0x{:06X}\"", track.color_code).unwrap();
    }

    out.push_str("/>\n");
}

/// Generate a complete Rekordbox-compatible XML file.
pub fn generate_xml(tracks: &[Track]) -> String {
    let mut out = String::with_capacity(tracks.len() * 512);

    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str("<DJ_PLAYLISTS Version=\"1.0.0\">\n");
    out.push_str("  <PRODUCT Name=\"rekordbox\" Version=\"7.2.10\" Company=\"AlphaTheta\"/>\n");
    write!(
        out,
        "  <COLLECTION Entries=\"{count}\">\n",
        count = tracks.len()
    )
    .unwrap();

    for (i, track) in tracks.iter().enumerate() {
        write_track(&mut out, track, i + 1);
    }

    out.push_str("  </COLLECTION>\n");
    out.push_str("</DJ_PLAYLISTS>\n");

    out
}

/// Write XML to a file at the given path, creating parent directories if needed.
pub fn write_xml(tracks: &[Track], path: &Path) -> Result<(), std::io::Error> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let xml = generate_xml(tracks);
    fs::write(path, xml)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_track() -> Track {
        Track {
            id: "t1".to_string(),
            title: "Archangel".to_string(),
            artist: "Burial".to_string(),
            album: "Untrue".to_string(),
            genre: "Dubstep".to_string(),
            bpm: 139.5,
            key: "Am".to_string(),
            rating: 4,
            comments: "iconic garage vocal".to_string(),
            color: String::new(),
            color_code: 0,
            label: "Hyperdub".to_string(),
            remixer: String::new(),
            year: 2007,
            length: 240,
            file_path: "/Users/vz/Music/Burial/Untrue/01 Archangel.flac".to_string(),
            play_count: 12,
            bit_rate: 1411,
            sample_rate: 44100,
            file_type: 5,
            date_added: "2023-01-15".to_string(),
        }
    }

    #[test]
    fn test_xml_escape() {
        assert_eq!(xml_escape("a & b"), "a &amp; b");
        assert_eq!(xml_escape("a < b"), "a &lt; b");
        assert_eq!(xml_escape("a > b"), "a &gt; b");
        assert_eq!(xml_escape("a \"b\""), "a &quot;b&quot;");
        assert_eq!(xml_escape("it's"), "it&apos;s");
        assert_eq!(xml_escape("no escaping"), "no escaping");
    }

    #[test]
    fn test_path_to_location() {
        assert_eq!(
            path_to_location("/Users/vz/Music/track.flac"),
            "file://localhost/Users/vz/Music/track.flac"
        );
        assert_eq!(
            path_to_location("/Users/vz/Music/my track.flac"),
            "file://localhost/Users/vz/Music/my%20track.flac"
        );
    }

    #[test]
    fn test_path_to_location_special_chars() {
        assert_eq!(
            path_to_location("/Users/vz/Music/Drum & Bass/track.flac"),
            "file://localhost/Users/vz/Music/Drum%20%26%20Bass/track.flac"
        );
    }

    #[test]
    fn test_generate_xml_structure() {
        let tracks = vec![make_test_track()];
        let xml = generate_xml(&tracks);

        assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
        assert!(xml.contains("<DJ_PLAYLISTS Version=\"1.0.0\">"));
        assert!(xml.contains("<PRODUCT Name=\"rekordbox\""));
        assert!(xml.contains("<COLLECTION Entries=\"1\">"));
        assert!(xml.contains("</COLLECTION>"));
        assert!(xml.contains("</DJ_PLAYLISTS>"));
    }

    #[test]
    fn test_generate_xml_track_attributes() {
        let tracks = vec![make_test_track()];
        let xml = generate_xml(&tracks);

        assert!(xml.contains("Name=\"Archangel\""));
        assert!(xml.contains("Artist=\"Burial\""));
        assert!(xml.contains("Genre=\"Dubstep\""));
        assert!(xml.contains("Rating=\"204\"")); // 4 stars = 204
        assert!(xml.contains("AverageBpm=\"139.50\""));
        assert!(xml.contains("Tonality=\"Am\""));
        assert!(xml.contains("Label=\"Hyperdub\""));
        assert!(xml.contains("Kind=\"FLAC File\""));
        assert!(xml.contains(
            "Location=\"file://localhost/Users/vz/Music/Burial/Untrue/01%20Archangel.flac\""
        ));
    }

    #[test]
    fn test_entries_count_matches() {
        let tracks = vec![make_test_track(), make_test_track()];
        let xml = generate_xml(&tracks);
        assert!(xml.contains("<COLLECTION Entries=\"2\">"));
        // Count actual TRACK elements
        let track_count = xml.matches("<TRACK ").count();
        assert_eq!(track_count, 2);
    }

    #[test]
    fn test_xml_escaping_in_track() {
        let mut track = make_test_track();
        track.artist = "Simon & Garfunkel".to_string();
        track.comments = "\"great\" <track>".to_string();
        let xml = generate_xml(&[track]);
        assert!(xml.contains("Artist=\"Simon &amp; Garfunkel\""));
        assert!(xml.contains("Comments=\"&quot;great&quot; &lt;track&gt;\""));
    }

    #[test]
    fn test_write_xml_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("output/test.xml");
        let tracks = vec![make_test_track()];
        write_xml(&tracks, &path).unwrap();
        assert!(path.exists());
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("<DJ_PLAYLISTS"));
    }

    // ==================== Integration tests (real DB) ====================

    fn load_real_tracks(limit: usize) -> Option<Vec<crate::types::Track>> {
        let conn = crate::db::open_real_db()?;
        let params = crate::db::SearchParams {
            query: None,
            artist: None,
            genre: None,
            rating_min: None,
            bpm_min: None,
            bpm_max: None,
            key: None,
            playlist: None,
            has_genre: None,
            exclude_samples: false,
            limit: Some(limit as u32),
        };
        Some(crate::db::search_tracks(&conn, &params).unwrap())
    }

    #[test]
    #[ignore]
    fn test_real_tracks_to_xml() {
        let tracks = load_real_tracks(100).expect("backup tarball not found");
        let xml = generate_xml(&tracks);

        // Structural checks
        assert!(xml.starts_with("<?xml"));
        assert!(xml.contains("<COLLECTION Entries=\"100\">"));
        let track_count = xml.matches("<TRACK ").count();
        assert_eq!(
            track_count, 100,
            "expected 100 TRACK elements, got {track_count}"
        );

        // No NUL or control characters (except newline/tab)
        for (i, c) in xml.chars().enumerate() {
            if c.is_control() && c != '\n' && c != '\r' && c != '\t' {
                panic!("control char U+{:04X} at position {i}", c as u32);
            }
        }
    }

    #[test]
    #[ignore]
    fn test_real_tracks_xml_well_formed() {
        let tracks = load_real_tracks(200).expect("backup tarball not found");
        let xml = generate_xml(&tracks);

        // Check no unescaped characters in attribute values.
        // Every attribute value is between quotes. Scan for patterns that indicate
        // broken escaping: =" followed by content with raw & < " before the closing "
        for line in xml.lines() {
            if !line.contains("<TRACK ") {
                continue;
            }
            // Simple check: no raw & that isn't an entity reference
            let mut in_attr = false;
            let chars: Vec<char> = line.chars().collect();
            let mut i = 0;
            while i < chars.len() {
                if chars[i] == '"' {
                    in_attr = !in_attr;
                } else if in_attr && chars[i] == '&' {
                    // Must be followed by amp; lt; gt; quot; apos; or #
                    let rest: String = chars[i + 1..].iter().take(6).collect();
                    assert!(
                        rest.starts_with("amp;")
                            || rest.starts_with("lt;")
                            || rest.starts_with("gt;")
                            || rest.starts_with("quot;")
                            || rest.starts_with("apos;")
                            || rest.starts_with('#'),
                        "unescaped & in TRACK line at pos {i}: ...{}...",
                        chars[i.saturating_sub(10)..chars.len().min(i + 20)]
                            .iter()
                            .collect::<String>()
                    );
                } else if in_attr && chars[i] == '<' {
                    panic!("unescaped < in attribute value at pos {i}");
                }
                i += 1;
            }
        }
    }

    #[test]
    #[ignore]
    fn test_real_tracks_xml_field_fidelity() {
        let tracks = load_real_tracks(50).expect("backup tarball not found");
        let xml = generate_xml(&tracks);

        for track in &tracks {
            let rating_xml = crate::types::stars_to_rating(track.rating);
            let expected_rating = format!("Rating=\"{rating_xml}\"");
            assert!(
                xml.contains(&expected_rating),
                "missing {} for track '{}'",
                expected_rating,
                track.title
            );

            // BPM format: X.XX
            let expected_bpm = format!("AverageBpm=\"{:.2}\"", track.bpm);
            assert!(
                xml.contains(&expected_bpm),
                "missing {} for track '{}'",
                expected_bpm,
                track.title
            );
        }

        // All Location values should start with file://localhost/
        for line in xml.lines() {
            if let Some(loc_start) = line.find("Location=\"") {
                let after = &line[loc_start + 10..];
                assert!(
                    after.starts_with("file://localhost/"),
                    "Location doesn't start with file://localhost/: {}",
                    &after[..after.len().min(60)]
                );
            }
        }
    }

    #[test]
    #[ignore]
    fn test_real_tracks_xml_write_and_read_back() {
        let tracks = load_real_tracks(50).expect("backup tarball not found");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("real_test.xml");

        write_xml(&tracks, &path).unwrap();
        assert!(path.exists());

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.starts_with("<?xml"));
        assert!(content.contains("<COLLECTION Entries=\"50\">"));

        // File should be reasonable size (roughly 500-2000 bytes per track)
        let size = std::fs::metadata(&path).unwrap().len();
        assert!(size > 50 * 200, "file too small: {size} bytes");
        assert!(size < 50 * 5000, "file too large: {size} bytes");
    }

    #[test]
    #[ignore]
    fn test_real_full_library_xml_generation() {
        let conn = crate::db::open_real_db().expect("backup tarball not found");

        // Load all tracks via paging
        let mut all = Vec::new();
        let page_size: u32 = 200;
        let mut offset: u32 = 0;
        loop {
            let sql = format!(
                "{}WHERE c.rb_local_deleted = 0 ORDER BY c.ID LIMIT {} OFFSET {}",
                crate::db::TRACK_SELECT,
                page_size,
                offset
            );
            let mut stmt = conn.prepare(&sql).unwrap();
            let batch: Vec<crate::types::Track> = stmt
                .query_map([], |row| crate::db::row_to_track(row))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            let count = batch.len();
            all.extend(batch);
            if (count as u32) < page_size {
                break;
            }
            offset += page_size;
        }

        assert!(all.len() > 2000, "expected >2000 tracks, got {}", all.len());

        let xml = generate_xml(&all);

        // Entries count matches
        let expected = format!("<COLLECTION Entries=\"{}\">", all.len());
        assert!(xml.contains(&expected), "Entries count mismatch");

        // TRACK element count matches
        let track_count = xml.matches("<TRACK ").count();
        assert_eq!(
            track_count,
            all.len(),
            "TRACK element count mismatch: {track_count} vs {}",
            all.len()
        );

        // Reasonable file size: ~500-3000 bytes per track
        let size = xml.len();
        assert!(
            size > all.len() * 200,
            "XML too small: {size} bytes for {} tracks",
            all.len()
        );
        assert!(
            size < all.len() * 5000,
            "XML too large: {size} bytes for {} tracks",
            all.len()
        );
    }
}
