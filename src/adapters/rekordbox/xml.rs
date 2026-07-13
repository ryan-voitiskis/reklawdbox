//! Rekordbox XML generation and transactional file-write adapter.

use std::collections::HashMap;
use std::fmt::Write;
use std::fs;
use std::io::Write as IoWrite;
use std::path::Path;

use crate::domain::library::Track;

const TARGET_REKORDBOX_VERSION: &str = "7.2.10";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaylistDef {
    pub name: String,
    pub track_ids: Vec<String>,
}

pub fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // XML 1.0 forbids most C0 control chars.
            c if c.is_control() && c != '\n' && c != '\r' && c != '\t' => {}
            _ => out.push(c),
        }
    }
    out
}

/// Convert a file system path to a Rekordbox Location URI.
/// e.g. `/Users/testuser/Music/file name.flac` → `file://localhost/Users/testuser/Music/file%20name.flac`
pub fn path_to_rekordbox_location_uri(file_path: &str) -> String {
    use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};

    let (uri_path, is_file_uri_input) =
        if let Some(path) = file_path.strip_prefix("file://localhost") {
            (path, true)
        } else if let Some(path) = file_path.strip_prefix("file://") {
            (path, true)
        } else {
            (file_path, false)
        };

    // Only URI inputs are percent-decoded first to avoid double-encoding URI segments.
    // Raw filesystem paths should keep literal '%' so it can be encoded as '%25'.
    let normalized_source = if is_file_uri_input {
        percent_decode_str(uri_path)
            .decode_utf8_lossy()
            .into_owned()
    } else {
        uri_path.to_string()
    };
    let mut normalized = normalized_source.replace('\\', "/");
    let bytes = normalized.as_bytes();
    let is_windows_drive = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    if is_windows_drive || !normalized.starts_with('/') {
        normalized.insert(0, '/');
    }

    const ENCODE_SET: &AsciiSet = &NON_ALPHANUMERIC
        .remove(b'-')
        .remove(b'_')
        .remove(b'.')
        .remove(b'~')
        .remove(b'/');

    let encoded = utf8_percent_encode(&normalized, ENCODE_SET).to_string();
    // Restore the colon for Windows drive letters (e.g. /C%3A/ → /C:/).
    // The colon is now encoded globally to avoid producing invalid URIs for
    // filenames that contain literal colons.
    let encoded = if is_windows_drive {
        encoded.replacen("%3A", ":", 1)
    } else {
        encoded
    };
    format!("file://localhost{encoded}")
}

fn write_collection_track(out: &mut String, track: &Track, track_id: usize) {
    let rating = crate::domain::library::stars_to_rating(track.rating);
    let location = xml_escape(&path_to_rekordbox_location_uri(&track.file_path));
    let kind = track.file_kind.as_kind_str();

    write!(
        out,
        "      <TRACK TrackID=\"{track_id}\" \
         Name=\"{name}\" \
         Artist=\"{artist}\" \
         Album=\"{album}\" \
         Genre=\"{genre}\" \
         Kind=\"{kind}\" \
         TotalTime=\"{total_time}\" \
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
         Label=\"{label}\"",
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

    if track.color_code > 0 {
        write!(out, " Colour=\"0x{:06X}\"", track.color_code).unwrap();
    }

    out.push_str("/>\n");
}

#[cfg(test)]
pub fn generate_xml(tracks: &[Track]) -> String {
    generate_xml_with_playlists(tracks, &[])
        .expect("playlist validation should not fail when no playlists are provided")
}

pub fn generate_xml_with_playlists(
    tracks: &[Track],
    playlists: &[PlaylistDef],
) -> Result<String, String> {
    let mut out = String::with_capacity(tracks.len() * 512);
    let mut track_id_map = HashMap::with_capacity(tracks.len());

    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str("<DJ_PLAYLISTS Version=\"1.0.0\">\n");
    writeln!(out, "  <PRODUCT Name=\"rekordbox\" Version=\"{TARGET_REKORDBOX_VERSION}\" Company=\"AlphaTheta\"/>").unwrap();
    writeln!(
        out,
        "  <COLLECTION Entries=\"{count}\">",
        count = tracks.len()
    )
    .unwrap();

    for (i, track) in tracks.iter().enumerate() {
        let xml_track_id = i + 1;
        track_id_map.insert(track.id.clone(), xml_track_id);
        write_collection_track(&mut out, track, xml_track_id);
    }

    out.push_str("  </COLLECTION>\n");

    if !playlists.is_empty() {
        out.push_str("  <PLAYLISTS>\n");
        writeln!(
            out,
            "    <NODE Type=\"0\" Name=\"ROOT\" Count=\"{}\">",
            playlists.len()
        )
        .unwrap();

        for playlist in playlists {
            writeln!(
                out,
                "      <NODE Type=\"1\" Name=\"{}\" Entries=\"{}\" KeyType=\"0\">",
                xml_escape(&playlist.name),
                playlist.track_ids.len()
            )
            .unwrap();

            for track_id in &playlist.track_ids {
                let xml_track_id = track_id_map.get(track_id).ok_or_else(|| {
                    format!(
                        "Playlist '{}' references unknown track ID '{}'",
                        playlist.name, track_id
                    )
                })?;
                writeln!(out, "        <TRACK Key=\"{xml_track_id}\"/>").unwrap();
            }

            out.push_str("      </NODE>\n");
        }

        out.push_str("    </NODE>\n");
        out.push_str("  </PLAYLISTS>\n");
    }
    out.push_str("</DJ_PLAYLISTS>\n");

    Ok(out)
}

#[cfg(test)]
pub fn write_xml(tracks: &[Track], path: &Path) -> Result<(), std::io::Error> {
    write_xml_with_playlists(tracks, &[], path)
}

pub fn write_xml_with_playlists(
    tracks: &[Track],
    playlists: &[PlaylistDef],
    path: &Path,
) -> Result<(), std::io::Error> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let xml = generate_xml_with_playlists(tracks, playlists)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let mut temp = tempfile::Builder::new()
        .prefix(".reklawdbox-xml-")
        .tempfile_in(parent)?;
    temp.write_all(xml.as_bytes())?;
    temp.as_file_mut().sync_all()?;
    temp.persist(path).map_err(|error| error.error)?;
    Ok(())
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
            file_path: "/Users/testuser/Music/Burial/Untrue/01 Archangel.flac".to_string(),
            play_count: 12,
            bit_rate: 1411,
            sample_rate: 44100,
            file_kind: crate::domain::library::FileKind::Flac,
            date_added: "2023-01-15".to_string(),
            position: None,
            played_at: None,
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
    fn test_xml_escape_filters_invalid_control_chars() {
        let escaped = xml_escape("ok\u{0000}\u{0008}\t\n\rtext");
        assert_eq!(escaped, "ok\t\n\rtext");
    }

    #[test]
    fn test_path_to_rekordbox_location_uri() {
        assert_eq!(
            path_to_rekordbox_location_uri("/Users/testuser/Music/track.flac"),
            "file://localhost/Users/testuser/Music/track.flac"
        );
        assert_eq!(
            path_to_rekordbox_location_uri("/Users/testuser/Music/my track.flac"),
            "file://localhost/Users/testuser/Music/my%20track.flac"
        );
    }

    #[test]
    fn test_path_to_rekordbox_location_uri_special_chars() {
        assert_eq!(
            path_to_rekordbox_location_uri("/Users/testuser/Music/Drum & Bass/track.flac"),
            "file://localhost/Users/testuser/Music/Drum%20%26%20Bass/track.flac"
        );
    }

    #[test]
    fn test_path_to_rekordbox_location_uri_windows_path() {
        assert_eq!(
            path_to_rekordbox_location_uri(r"C:\Users\testuser\Music\my track.flac"),
            "file://localhost/C:/Users/testuser/Music/my%20track.flac"
        );
    }

    #[test]
    fn test_path_to_rekordbox_location_uri_encodes_literal_percent_triplets_in_raw_paths() {
        assert_eq!(
            path_to_rekordbox_location_uri("/Users/testuser/Music/my%20track.flac"),
            "file://localhost/Users/testuser/Music/my%2520track.flac"
        );
    }

    #[test]
    fn test_path_to_rekordbox_location_uri_accepts_file_uri_input() {
        assert_eq!(
            path_to_rekordbox_location_uri("file://localhost/Users/testuser/Music/my%20track.flac"),
            "file://localhost/Users/testuser/Music/my%20track.flac"
        );
    }

    #[test]
    fn test_path_to_rekordbox_location_uri_encodes_colon_in_filename() {
        assert_eq!(
            path_to_rekordbox_location_uri("/Users/testuser/Music/Title: Subtitle.flac"),
            "file://localhost/Users/testuser/Music/Title%3A%20Subtitle.flac"
        );
    }

    #[test]
    fn test_path_to_rekordbox_location_uri_windows_drive_colon_preserved() {
        // Windows drive letter colon is preserved, but a colon in the filename is encoded.
        assert_eq!(
            path_to_rekordbox_location_uri(r"C:\Music\A:B.flac"),
            "file://localhost/C:/Music/A%3AB.flac"
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
            "Location=\"file://localhost/Users/testuser/Music/Burial/Untrue/01%20Archangel.flac\""
        ));
    }

    #[test]
    fn test_generate_xml_omits_unknown_round_trip_fields() {
        let tracks = vec![make_test_track()];
        let xml = generate_xml(&tracks);

        for forbidden in [
            "Composer=",
            "Grouping=",
            "Size=",
            "DiscNumber=",
            "TrackNumber=",
            "Mix=",
        ] {
            assert!(
                !xml.contains(forbidden),
                "XML should omit '{forbidden}' until those fields are loaded from Rekordbox"
            );
        }
    }

    #[test]
    fn test_entries_count_matches() {
        let tracks = vec![make_test_track(), make_test_track()];
        let xml = generate_xml(&tracks);
        assert!(xml.contains("<COLLECTION Entries=\"2\">"));
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
    fn test_generate_xml_with_playlists_structure_and_key_mapping() {
        let mut t1 = make_test_track();
        t1.id = "db-id-1".to_string();
        t1.title = "Track 1".to_string();

        let mut t2 = make_test_track();
        t2.id = "db-id-2".to_string();
        t2.title = "Track 2".to_string();

        let mut t3 = make_test_track();
        t3.id = "db-id-3".to_string();
        t3.title = "Track 3".to_string();

        let playlists = vec![PlaylistDef {
            name: "Set 2026-02-21 & Techno".to_string(),
            track_ids: vec![
                "db-id-2".to_string(),
                "db-id-1".to_string(),
                "db-id-3".to_string(),
            ],
        }];

        let xml = generate_xml_with_playlists(&[t1, t2, t3], &playlists)
            .expect("playlist XML should generate");

        assert!(xml.contains("<COLLECTION Entries=\"3\">"));
        assert!(xml.contains("<PLAYLISTS>"));
        assert!(xml.contains("<NODE Type=\"0\" Name=\"ROOT\" Count=\"1\">"));
        assert!(
            xml.contains(
                "<NODE Type=\"1\" Name=\"Set 2026-02-21 &amp; Techno\" Entries=\"3\" KeyType=\"0\">"
            ),
            "playlist name should be XML escaped"
        );

        let key2 = xml.find("<TRACK Key=\"2\"/>").expect("key 2 should exist");
        let key1 = xml.find("<TRACK Key=\"1\"/>").expect("key 1 should exist");
        let key3 = xml.find("<TRACK Key=\"3\"/>").expect("key 3 should exist");
        assert!(
            key2 < key1 && key1 < key3,
            "playlist key order should match input track_ids order"
        );
    }

    #[test]
    fn test_generate_xml_with_playlists_errors_for_unknown_track_id() {
        let track = make_test_track();
        let playlists = vec![PlaylistDef {
            name: "Bad Playlist".to_string(),
            track_ids: vec!["missing-id".to_string()],
        }];

        let err = generate_xml_with_playlists(&[track], &playlists)
            .expect_err("missing track references should fail");
        assert!(err.contains("missing-id"));
    }

    #[test]
    fn test_generate_xml_with_multiple_playlists_sets_root_count() {
        let mut t1 = make_test_track();
        t1.id = "db-id-1".to_string();
        t1.title = "Track 1".to_string();

        let mut t2 = make_test_track();
        t2.id = "db-id-2".to_string();
        t2.title = "Track 2".to_string();

        let mut t3 = make_test_track();
        t3.id = "db-id-3".to_string();
        t3.title = "Track 3".to_string();

        let playlists = vec![
            PlaylistDef {
                name: "First Set".to_string(),
                track_ids: vec!["db-id-1".to_string(), "db-id-3".to_string()],
            },
            PlaylistDef {
                name: "Second & Set".to_string(),
                track_ids: vec!["db-id-2".to_string()],
            },
        ];

        let xml = generate_xml_with_playlists(&[t1, t2, t3], &playlists)
            .expect("playlist XML should generate");

        assert!(xml.contains("<NODE Type=\"0\" Name=\"ROOT\" Count=\"2\">"));
        assert_eq!(xml.matches("<NODE Type=\"1\"").count(), 2);
        assert!(xml.contains("<NODE Type=\"1\" Name=\"First Set\" Entries=\"2\" KeyType=\"0\">"));
        assert!(
            xml.contains("<NODE Type=\"1\" Name=\"Second &amp; Set\" Entries=\"1\" KeyType=\"0\">")
        );

        let first_start = xml
            .find("<NODE Type=\"1\" Name=\"First Set\" Entries=\"2\" KeyType=\"0\">")
            .expect("first playlist should exist");
        let first_end = first_start
            + xml[first_start..]
                .find("</NODE>")
                .expect("first playlist should close");
        let first_block = &xml[first_start..first_end];
        let first_key1 = first_block
            .find("<TRACK Key=\"1\"/>")
            .expect("first playlist should reference key 1");
        let first_key3 = first_block
            .find("<TRACK Key=\"3\"/>")
            .expect("first playlist should reference key 3");
        assert!(
            first_key1 < first_key3,
            "first playlist key ordering should match input order"
        );

        let second_start = xml
            .find("<NODE Type=\"1\" Name=\"Second &amp; Set\" Entries=\"1\" KeyType=\"0\">")
            .expect("second playlist should exist");
        let second_end = second_start
            + xml[second_start..]
                .find("</NODE>")
                .expect("second playlist should close");
        let second_block = &xml[second_start..second_end];
        assert!(
            second_block.contains("<TRACK Key=\"2\"/>"),
            "second playlist should reference key 2"
        );
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

    #[test]
    fn write_xml_atomically_replaces_existing_file_without_temp_artifacts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("export.xml");
        std::fs::write(&path, "stale export").unwrap();

        write_xml(&[make_test_track()], &path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.starts_with("<?xml"));
        assert!(!content.contains("stale export"));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    // ==================== Integration tests (real DB) ====================

    fn load_real_tracks(limit: usize) -> Option<Vec<crate::domain::library::Track>> {
        let conn = crate::adapters::rekordbox::open_real_db()?;
        let params = crate::adapters::rekordbox::SearchParams {
            query: None,
            artist: None,
            genre: None,
            rating_min: None,
            bpm_min: None,
            bpm_max: None,
            key: None,
            playlist: None,
            has_genre: None,
            has_label: None,
            year_zero: None,
            label: None,
            path: None,
            path_prefix: None,
            added_after: None,
            added_before: None,
            exclude_samples: false,
            limit: Some(limit as u32),
            offset: None,
        };
        Some(crate::adapters::rekordbox::search_tracks(&conn, &params).unwrap())
    }

    #[test]
    #[ignore]
    fn test_real_tracks_to_xml() {
        let tracks = load_real_tracks(100).expect("backup tarball not found");
        let xml = generate_xml(&tracks);

        assert!(xml.starts_with("<?xml"));
        assert!(xml.contains("<COLLECTION Entries=\"100\">"));
        let track_count = xml.matches("<TRACK ").count();
        assert_eq!(
            track_count, 100,
            "expected 100 TRACK elements, got {track_count}"
        );

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

        for line in xml.lines() {
            if !line.contains("<TRACK ") {
                continue;
            }
            let mut in_attr = false;
            let chars: Vec<char> = line.chars().collect();
            let mut i = 0;
            while i < chars.len() {
                if chars[i] == '"' {
                    in_attr = !in_attr;
                } else if in_attr && chars[i] == '&' {
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
            let rating_xml = crate::domain::library::stars_to_rating(track.rating);
            let expected_rating = format!("Rating=\"{rating_xml}\"");
            assert!(
                xml.contains(&expected_rating),
                "missing {} for track '{}'",
                expected_rating,
                track.title
            );

            let expected_bpm = format!("AverageBpm=\"{:.2}\"", track.bpm);
            assert!(
                xml.contains(&expected_bpm),
                "missing {} for track '{}'",
                expected_bpm,
                track.title
            );
        }

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

        let size = std::fs::metadata(&path).unwrap().len();
        assert!(size > 50 * 200, "file too small: {size} bytes");
        assert!(size < 50 * 5000, "file too large: {size} bytes");
    }

    #[test]
    #[ignore]
    fn test_real_full_library_xml_generation() {
        let conn = crate::adapters::rekordbox::open_real_db().expect("backup tarball not found");

        let mut all = Vec::new();
        let page_size: u32 = 200;
        let mut offset: u32 = 0;
        loop {
            let sql = format!(
                "{}WHERE c.rb_local_deleted = 0 ORDER BY c.ID LIMIT {} OFFSET {}",
                crate::adapters::rekordbox::TRACK_SELECT,
                page_size,
                offset
            );
            let mut stmt = conn.prepare(&sql).unwrap();
            let batch: Vec<crate::domain::library::Track> = stmt
                .query_map([], crate::adapters::rekordbox::row_to_track)
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

        let expected = format!("<COLLECTION Entries=\"{}\">", all.len());
        assert!(xml.contains(&expected), "Entries count mismatch");

        let track_count = xml.matches("<TRACK ").count();
        assert_eq!(
            track_count,
            all.len(),
            "TRACK element count mismatch: {track_count} vs {}",
            all.len()
        );

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
