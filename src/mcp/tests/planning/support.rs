use rusqlite::{Connection, params};
use tempfile::TempDir;

use crate::domain::planning::{
    EnergyPhase, HarmonicMixingStyle, PriorityWeights, TrackProfile, TransitionMixingPolicy,
    TransitionMoment, genre_family_for, parse_camelot_key,
};

use super::super::common::{
    create_single_track_test_db, set_test_audio_analysis, write_test_audio_file,
};

pub(super) fn create_build_set_test_db() -> (Connection, Vec<String>, TempDir) {
    let audio_dir = tempfile::tempdir().expect("build_set temp audio dir should create");
    let first_track_path = audio_dir.path().join("set-track-1.flac");
    let conn = create_single_track_test_db(
        "set-track-1",
        first_track_path
            .to_str()
            .expect("first build_set track path should be UTF-8"),
    );
    conn.execute_batch(
        "
            INSERT INTO djmdGenre (ID, Name) VALUES ('g2', 'House');
            INSERT INTO djmdGenre (ID, Name) VALUES ('g3', 'Tech House');

            INSERT INTO djmdKey (ID, ScaleName) VALUES ('k2', 'Em');
            INSERT INTO djmdKey (ID, ScaleName) VALUES ('k3', 'Bm');
            INSERT INTO djmdKey (ID, ScaleName) VALUES ('k4', 'F#m');
            INSERT INTO djmdKey (ID, ScaleName) VALUES ('k5', 'C#m');
            INSERT INTO djmdKey (ID, ScaleName) VALUES ('k6', 'Dm');
            ",
    )
    .expect("build_set fixture taxonomy inserts should succeed");

    let tracks: [(&str, &str, &str, &str, i32, i32); 5] = [
        ("set-track-2", "Second Step", "g1", "k2", 12400, 300),
        ("set-track-3", "Third Wave", "g2", "k3", 12600, 0),
        ("set-track-4", "Fourth Lift", "g3", "k4", 12800, 360),
        ("set-track-5", "Fifth Peak", "g3", "k5", 12950, 420),
        ("set-track-6", "Sixth Release", "g2", "k6", 12350, 250),
    ];

    for (index, (track_id, title, genre_id, key_id, bpm, length)) in tracks.iter().enumerate() {
        conn.execute(
            "INSERT INTO djmdContent (
                    ID, Title, ArtistID, AlbumID, GenreID, KeyID, ColorID, LabelID, RemixerID,
                    BPM, Rating, Commnt, ReleaseYear, Length, FolderPath, DJPlayCount, BitRate,
                    SampleRate, FileType, created_at, rb_local_deleted
                ) VALUES (
                    ?1, ?2, 'a1', 'al1', ?3, ?4, 'c1', 'l1', '',
                    ?5, 153, 'build_set fixture', 2025, ?6, ?7, '0', 1411,
                    44100, 5, '2025-01-03', 0
                )",
            params![
                *track_id,
                *title,
                *genre_id,
                *key_id,
                *bpm,
                *length,
                audio_dir
                    .path()
                    .join(format!("{track_id}.flac"))
                    .to_string_lossy()
                    .to_string(),
            ],
        )
        .unwrap_or_else(|e| panic!("fixture track insert {index} should succeed: {e}"));
    }

    (
        conn,
        vec![
            "set-track-1".to_string(),
            "set-track-2".to_string(),
            "set-track-3".to_string(),
            "set-track-4".to_string(),
            "set-track-5".to_string(),
            "set-track-6".to_string(),
        ],
        audio_dir,
    )
}

pub(super) fn seed_build_set_cache(store_conn: &Connection, audio_dir: &std::path::Path) {
    let rows: [(&str, f64, &str, f64); 6] = [
        ("set-track-1.flac", 122.0, "8A", 1.02),
        ("set-track-2.flac", 124.0, "9A", 1.20),
        ("set-track-3.flac", 126.0, "10A", 1.44),
        ("set-track-4.flac", 128.0, "11A", 1.80),
        ("set-track-5.flac", 130.0, "12A", 2.22),
        ("set-track-6.flac", 123.5, "7A", 1.26),
    ];

    for (index, (file_name, bpm, key_camelot, danceability)) in rows.iter().enumerate() {
        let path = audio_dir.join(file_name);
        let (file_size, file_mtime) = write_test_audio_file(&path, 1000 + index);
        let stratum = serde_json::json!({
            "bpm": *bpm,
            "key": "Am",
            "key_camelot": *key_camelot,
            "analyzer_version": "stratum-dsp-test"
        });
        let essentia = serde_json::json!({
            "danceability": *danceability,
            "loudness_integrated": -18.0 + (*danceability * 4.0),
            "onset_rate": 2.5 + (*danceability * 2.0),
            "analyzer_version": "essentia-test"
        });
        set_test_audio_analysis(
            store_conn,
            path.to_str().expect("seed path should be UTF-8"),
            "stratum-dsp",
            file_size,
            file_mtime,
            "stratum-dsp-test",
            &stratum.to_string(),
        )
        .unwrap_or_else(|e| panic!("stratum cache seed {index} should succeed: {e}"));
        set_test_audio_analysis(
            store_conn,
            path.to_str().expect("seed path should be UTF-8"),
            "essentia",
            file_size,
            file_mtime,
            "essentia-test",
            &essentia.to_string(),
        )
        .unwrap_or_else(|e| panic!("essentia cache seed {index} should succeed: {e}"));
    }
}

pub(super) fn make_test_profile(
    id: &str,
    key: &str,
    bpm: f64,
    energy: f64,
    genre: &str,
) -> TrackProfile {
    TrackProfile {
        track: crate::domain::library::Track {
            id: id.to_string(),
            title: id.to_string(),
            artist: "Test".to_string(),
            album: String::new(),
            genre: genre.to_string(),
            key: key.to_string(),
            bpm,
            rating: 0,
            comments: String::new(),
            color: String::new(),
            color_code: 0,
            label: String::new(),
            remixer: String::new(),
            year: 0,
            length: 300,
            file_path: format!("/tmp/{id}.flac"),
            play_count: 0,
            bit_rate: 1411,
            sample_rate: 44100,
            file_kind: crate::domain::library::FileKind::Flac,
            date_added: String::new(),
            position: None,
            played_at: None,
        },
        camelot_key: parse_camelot_key(key),
        key_display: key.to_string(),
        bpm,
        energy,
        brightness: None,
        rhythm_regularity: None,
        loudness_range: None,
        canonical_genre: Some(genre.to_string()),
        genre_family: genre_family_for(genre),
        timbral: None,
    }
}

pub(super) fn mixing_policy(
    weights: &PriorityWeights,
    master_tempo: bool,
    harmonic_style: Option<HarmonicMixingStyle>,
) -> TransitionMixingPolicy<'_> {
    TransitionMixingPolicy {
        weights,
        master_tempo,
        harmonic_style,
    }
}

pub(super) fn transition_moment(
    from_phase: Option<EnergyPhase>,
    to_phase: Option<EnergyPhase>,
    genre_run_length: u32,
    play_bpms: Option<(f64, f64)>,
) -> TransitionMoment {
    TransitionMoment {
        from_phase,
        to_phase,
        genre_run_length,
        play_bpms,
    }
}
