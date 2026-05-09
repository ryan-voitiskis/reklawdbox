//! Rekordbox ANLZ binary file parser.
//!
//! Currently parses the `PQTZ` (beat grid) tag from `ANLZ0000.DAT`. The grid
//! is stored as a dense list of beats with millisecond timestamps and
//! per-beat BPM (so variable-tempo grids edited in the Rekordbox GUI are
//! preserved). The position-within-bar is stored as `bar_position` 1..=4
//! where 1 marks a downbeat.
//!
//! ANLZ files live under `~/Library/Pioneer/rekordbox/share/PIONEER/USBANLZ/`,
//! addressed per-track by `djmdContent.AnalysisDataPath`.
//!
//! Not yet wired into `analyze_with_stratum` — the public functions are
//! currently used only by tests and the dub-stab investigation example.

#![allow(dead_code)]

use std::fs;
use std::path::Path;

use stratum_dsp::analysis::result::BeatGrid;

#[derive(Debug, thiserror::Error)]
pub enum AnlzError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid ANLZ file: {0}")]
    Invalid(String),
    #[error("PQTZ tag not found in {0}")]
    NoPqtz(String),
}

/// One row of the dense PQTZ beat list.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PqtzBeat {
    /// 1..=4 within the bar; 1 marks a downbeat.
    pub bar_position: u8,
    pub bpm: f32,
    pub time_seconds: f32,
}

const FILE_HEADER_LEN: usize = 28;
const TAG_ENVELOPE_LEN: usize = 12;
const PQTZ_ENTRY_LEN: usize = 8;
const PQTZ_HEADER_LEN: usize = 24;

/// Read the dense PQTZ beat list from an `ANLZ0000.DAT` file.
pub fn read_pqtz_beats(path: &Path) -> Result<Vec<PqtzBeat>, AnlzError> {
    let bytes = fs::read(path)?;
    parse_pqtz_beats(&bytes).map_err(|e| match e {
        AnlzError::NoPqtz(_) => AnlzError::NoPqtz(path.display().to_string()),
        other => other,
    })
}

/// Read the beat grid from an `ANLZ0000.DAT` file as `BeatGrid` for use with
/// `stratum_dsp::features::dub_stab`. Bars and downbeats both come from the
/// `bar_position == 1` rows.
pub fn read_beat_grid(path: &Path) -> Result<BeatGrid, AnlzError> {
    let raw = read_pqtz_beats(path)?;
    let beats: Vec<f32> = raw.iter().map(|b| b.time_seconds).collect();
    let bars: Vec<f32> = raw
        .iter()
        .filter(|b| b.bar_position == 1)
        .map(|b| b.time_seconds)
        .collect();
    Ok(BeatGrid {
        downbeats: bars.clone(),
        beats,
        bars,
    })
}

/// Map a `djmdContent.AnalysisDataPath` (e.g. `"/PIONEER/USBANLZ/<hex>/<uuid>/ANLZ0000.DAT"`)
/// to an absolute filesystem path under the Rekordbox share root.
///
/// Honours `REKORDBOX_ANLZ_ROOT` (mainly for tests) before falling back to
/// `~/Library/Pioneer/rekordbox/share`.
pub fn resolve_anlz_path(analysis_data_path: &str) -> Option<String> {
    if let Ok(root) = std::env::var("REKORDBOX_ANLZ_ROOT") {
        return Some(format!("{root}{analysis_data_path}"));
    }
    let home = std::env::var("HOME").ok()?;
    Some(format!(
        "{home}/Library/Pioneer/rekordbox/share{analysis_data_path}"
    ))
}

fn read_u32_be(bytes: &[u8], off: usize) -> Result<u32, AnlzError> {
    bytes
        .get(off..off + 4)
        .ok_or_else(|| AnlzError::Invalid(format!("truncated at offset {off}")))
        .map(|s| u32::from_be_bytes(s.try_into().unwrap()))
}

fn read_u16_be(bytes: &[u8], off: usize) -> Result<u16, AnlzError> {
    bytes
        .get(off..off + 2)
        .ok_or_else(|| AnlzError::Invalid(format!("truncated at offset {off}")))
        .map(|s| u16::from_be_bytes(s.try_into().unwrap()))
}

fn parse_pqtz_beats(bytes: &[u8]) -> Result<Vec<PqtzBeat>, AnlzError> {
    if bytes.len() < FILE_HEADER_LEN {
        return Err(AnlzError::Invalid("file shorter than ANLZ header".into()));
    }
    if &bytes[..4] != b"PMAI" {
        return Err(AnlzError::Invalid(format!(
            "expected 'PMAI' magic, found {:?}",
            &bytes[..4]
        )));
    }

    let mut off = FILE_HEADER_LEN;
    while off + TAG_ENVELOPE_LEN <= bytes.len() {
        let fourcc = &bytes[off..off + 4];
        let len_tag = read_u32_be(bytes, off + 8)? as usize;
        if len_tag < TAG_ENVELOPE_LEN {
            return Err(AnlzError::Invalid(format!(
                "tag at offset {off} has impossible len_tag={len_tag}"
            )));
        }
        let tag_end = off
            .checked_add(len_tag)
            .ok_or_else(|| AnlzError::Invalid("tag length overflows".into()))?;
        if tag_end > bytes.len() {
            return Err(AnlzError::Invalid(format!(
                "tag at offset {off} extends past file end"
            )));
        }
        if fourcc == b"PQTZ" {
            return parse_pqtz_body(&bytes[off..tag_end]);
        }
        off = tag_end;
    }
    Err(AnlzError::NoPqtz(String::new()))
}

fn parse_pqtz_body(tag: &[u8]) -> Result<Vec<PqtzBeat>, AnlzError> {
    if tag.len() < PQTZ_HEADER_LEN {
        return Err(AnlzError::Invalid(
            "PQTZ tag shorter than its header".into(),
        ));
    }
    let count = read_u32_be(tag, 20)? as usize;
    let needed = PQTZ_HEADER_LEN
        .checked_add(count.checked_mul(PQTZ_ENTRY_LEN).ok_or_else(|| {
            AnlzError::Invalid(format!("PQTZ entry_count {count} would overflow"))
        })?)
        .ok_or_else(|| AnlzError::Invalid("PQTZ size overflow".into()))?;
    if tag.len() < needed {
        return Err(AnlzError::Invalid(format!(
            "PQTZ entries truncated: need {needed} bytes, have {}",
            tag.len()
        )));
    }
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let p = PQTZ_HEADER_LEN + i * PQTZ_ENTRY_LEN;
        let raw_position = read_u16_be(tag, p)?;
        if !(1..=4).contains(&raw_position) {
            // Reject rather than truncate to u8: a value of 257 would wrap to
            // 1 and inject a phantom downbeat into the bars list.
            return Err(AnlzError::Invalid(format!(
                "beat[{i}] bar_position={raw_position} out of 1..=4"
            )));
        }
        let bpm_x100 = read_u16_be(tag, p + 2)?;
        let time_ms = read_u32_be(tag, p + 4)?;
        out.push(PqtzBeat {
            bar_position: raw_position as u8,
            bpm: bpm_x100 as f32 / 100.0,
            time_seconds: time_ms as f32 / 1000.0,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Synthesise a minimal ANLZ file with one PPTH (string) tag and one PQTZ
    /// tag carrying `entries` beats. Used in unit tests so we don't require
    /// real Rekordbox files.
    fn synth_anlz(entries: &[PqtzBeat]) -> Vec<u8> {
        let mut buf = Vec::new();
        // File header (28 bytes): magic, len_header, len_file, 4 unknowns
        buf.extend_from_slice(b"PMAI");
        buf.extend_from_slice(&28u32.to_be_bytes());
        let len_file_off = buf.len();
        buf.extend_from_slice(&0u32.to_be_bytes()); // patched below
        for _ in 0..4 {
            buf.extend_from_slice(&0u32.to_be_bytes());
        }

        // Decoy tag (PPTH) so we exercise the tag-walk loop.
        buf.extend_from_slice(b"PPTH");
        buf.extend_from_slice(&16u32.to_be_bytes()); // len_header
        let pptz_payload = b"x\0\0\0";
        let pptz_total = TAG_ENVELOPE_LEN + 4 + pptz_payload.len();
        buf.extend_from_slice(&(pptz_total as u32).to_be_bytes()); // len_tag
        buf.extend_from_slice(&0u32.to_be_bytes());
        buf.extend_from_slice(pptz_payload);

        // PQTZ tag.
        buf.extend_from_slice(b"PQTZ");
        buf.extend_from_slice(&24u32.to_be_bytes()); // len_header
        let pqtz_total = TAG_ENVELOPE_LEN + 12 + entries.len() * PQTZ_ENTRY_LEN;
        buf.extend_from_slice(&(pqtz_total as u32).to_be_bytes());
        buf.extend_from_slice(&0u32.to_be_bytes()); // pad
        buf.extend_from_slice(&0x00080000u32.to_be_bytes()); // const
        buf.extend_from_slice(&(entries.len() as u32).to_be_bytes());
        for e in entries {
            buf.extend_from_slice(&(e.bar_position as u16).to_be_bytes());
            buf.extend_from_slice(&((e.bpm * 100.0).round() as u16).to_be_bytes());
            buf.extend_from_slice(&((e.time_seconds * 1000.0).round() as u32).to_be_bytes());
        }

        // Patch len_file.
        let len_file = buf.len() as u32;
        buf[len_file_off..len_file_off + 4].copy_from_slice(&len_file.to_be_bytes());
        buf
    }

    #[test]
    fn parses_synthetic_pqtz_round_trips_entries() {
        let entries = vec![
            PqtzBeat {
                bar_position: 4,
                bpm: 122.0,
                time_seconds: 0.050,
            },
            PqtzBeat {
                bar_position: 1,
                bpm: 122.0,
                time_seconds: 0.542,
            },
            PqtzBeat {
                bar_position: 2,
                bpm: 122.0,
                time_seconds: 1.034,
            },
        ];
        let bytes = synth_anlz(&entries);
        let parsed = parse_pqtz_beats(&bytes).expect("parse");
        assert_eq!(parsed.len(), entries.len());
        for (got, want) in parsed.iter().zip(entries.iter()) {
            assert_eq!(got.bar_position, want.bar_position);
            assert!((got.bpm - want.bpm).abs() < 0.01);
            assert!((got.time_seconds - want.time_seconds).abs() < 1e-3);
        }
    }

    #[test]
    fn read_beat_grid_separates_downbeats_into_bars() {
        let entries = vec![
            PqtzBeat {
                bar_position: 4,
                bpm: 120.0,
                time_seconds: 0.000,
            },
            PqtzBeat {
                bar_position: 1,
                bpm: 120.0,
                time_seconds: 0.500,
            },
            PqtzBeat {
                bar_position: 2,
                bpm: 120.0,
                time_seconds: 1.000,
            },
            PqtzBeat {
                bar_position: 3,
                bpm: 120.0,
                time_seconds: 1.500,
            },
            PqtzBeat {
                bar_position: 4,
                bpm: 120.0,
                time_seconds: 2.000,
            },
            PqtzBeat {
                bar_position: 1,
                bpm: 120.0,
                time_seconds: 2.500,
            },
        ];
        let bytes = synth_anlz(&entries);
        let parsed = parse_pqtz_beats(&bytes).expect("parse");
        let beats: Vec<f32> = parsed.iter().map(|b| b.time_seconds).collect();
        let bars: Vec<f32> = parsed
            .iter()
            .filter(|b| b.bar_position == 1)
            .map(|b| b.time_seconds)
            .collect();
        assert_eq!(beats.len(), 6);
        assert_eq!(bars, vec![0.500, 2.500]);
    }

    #[test]
    fn rejects_bar_position_out_of_range() {
        // Hand-craft a PQTZ tag with bar_position = 257. Without the
        // u16-vs-u8 check, this would silently wrap to 1 and look like a
        // downbeat.
        let mut tag = Vec::new();
        tag.extend_from_slice(b"PQTZ");
        tag.extend_from_slice(&24u32.to_be_bytes());
        let total = TAG_ENVELOPE_LEN + 12 + PQTZ_ENTRY_LEN;
        tag.extend_from_slice(&(total as u32).to_be_bytes());
        tag.extend_from_slice(&0u32.to_be_bytes());
        tag.extend_from_slice(&0x00080000u32.to_be_bytes());
        tag.extend_from_slice(&1u32.to_be_bytes()); // count = 1
        tag.extend_from_slice(&257u16.to_be_bytes()); // bar_position
        tag.extend_from_slice(&12000u16.to_be_bytes()); // bpm
        tag.extend_from_slice(&0u32.to_be_bytes()); // time
        match parse_pqtz_body(&tag) {
            Err(AnlzError::Invalid(msg)) => assert!(msg.contains("bar_position"), "msg={msg}"),
            other => panic!("expected Invalid(bar_position), got {other:?}"),
        }
    }

    #[test]
    fn missing_pqtz_returns_no_pqtz_error() {
        // File with only the file header and a PPTH decoy.
        let bytes = synth_anlz_no_pqtz();
        match parse_pqtz_beats(&bytes) {
            Err(AnlzError::NoPqtz(_)) => {}
            other => panic!("expected NoPqtz, got {other:?}"),
        }
    }

    #[test]
    fn rejects_bad_magic() {
        let mut bytes = vec![0u8; FILE_HEADER_LEN + 10];
        bytes[0..4].copy_from_slice(b"NOPE");
        match parse_pqtz_beats(&bytes) {
            Err(AnlzError::Invalid(msg)) => assert!(msg.contains("PMAI")),
            other => panic!("expected Invalid(PMAI), got {other:?}"),
        }
    }

    #[test]
    fn rejects_zero_length_tag() {
        let mut bytes = vec![0u8; FILE_HEADER_LEN];
        bytes[..4].copy_from_slice(b"PMAI");
        // Tag with len_tag = 0 — would loop forever if not rejected.
        bytes.extend_from_slice(b"FAKE");
        bytes.extend_from_slice(&0u32.to_be_bytes());
        bytes.extend_from_slice(&0u32.to_be_bytes());
        match parse_pqtz_beats(&bytes) {
            Err(AnlzError::Invalid(msg)) => assert!(msg.contains("len_tag")),
            other => panic!("expected Invalid(len_tag), got {other:?}"),
        }
    }

    #[test]
    fn rejects_truncated_pqtz_entries() {
        // Direct call to parse_pqtz_body with a body whose count says 2
        // entries but only 1 entry's worth of bytes follow. The whole-file
        // walker would already have rejected this at the envelope level;
        // here we want to specifically exercise the entries-truncation
        // branch.
        let mut tag = Vec::new();
        tag.extend_from_slice(b"PQTZ");
        tag.extend_from_slice(&24u32.to_be_bytes()); // len_header
        tag.extend_from_slice(
            &(TAG_ENVELOPE_LEN as u32 + 12 + PQTZ_ENTRY_LEN as u32).to_be_bytes(),
        );
        tag.extend_from_slice(&0u32.to_be_bytes()); // pad
        tag.extend_from_slice(&0x00080000u32.to_be_bytes());
        tag.extend_from_slice(&2u32.to_be_bytes()); // count claims 2
        // Only 1 entry follows.
        tag.extend_from_slice(&[0; PQTZ_ENTRY_LEN]);
        match parse_pqtz_body(&tag) {
            Err(AnlzError::Invalid(msg)) => assert!(msg.contains("PQTZ"), "msg={msg}"),
            other => panic!("expected Invalid(PQTZ entries truncated), got {other:?}"),
        }
    }

    #[test]
    fn resolve_path_honours_env_root() {
        let prev = std::env::var("REKORDBOX_ANLZ_ROOT").ok();
        // SAFETY: tests share env; we restore below. Run in cargo's
        // single-thread test harness via `--test-threads=1` if needed.
        unsafe { std::env::set_var("REKORDBOX_ANLZ_ROOT", "/tmp/fake") };
        let p = resolve_anlz_path("/PIONEER/USBANLZ/aa/bb/ANLZ0000.DAT").unwrap();
        assert_eq!(p, "/tmp/fake/PIONEER/USBANLZ/aa/bb/ANLZ0000.DAT");
        unsafe {
            match prev {
                Some(v) => std::env::set_var("REKORDBOX_ANLZ_ROOT", v),
                None => std::env::remove_var("REKORDBOX_ANLZ_ROOT"),
            }
        }
    }

    /// End-to-end parse of a real ANLZ file. Gated on `REKORDBOX_ANLZ_TEST`
    /// pointing to an `ANLZ0000.DAT` so the default suite stays portable.
    #[test]
    fn parses_real_anlz_when_env_set() {
        let Ok(path) = std::env::var("REKORDBOX_ANLZ_TEST") else {
            eprintln!("REKORDBOX_ANLZ_TEST unset — skipping real-file parse");
            return;
        };
        let beats = read_pqtz_beats(Path::new(&path)).expect("parse real ANLZ");
        assert!(!beats.is_empty(), "real ANLZ should have beats");
        // Bar positions must be in 1..=4.
        for (i, b) in beats.iter().enumerate() {
            assert!(
                (1..=4).contains(&b.bar_position),
                "beat[{i}] bar_position={} out of range",
                b.bar_position
            );
        }
        // Times strictly ascending.
        for w in beats.windows(2) {
            assert!(
                w[1].time_seconds > w[0].time_seconds,
                "non-monotonic at {}s -> {}s",
                w[0].time_seconds,
                w[1].time_seconds
            );
        }
        // BPM sane.
        for b in &beats {
            assert!(
                (40.0..=300.0).contains(&b.bpm),
                "bpm={} out of plausible range",
                b.bpm
            );
        }
        eprintln!(
            "real ANLZ parsed: {} beats, first {} s, last {} s, modal bpm ≈ {}",
            beats.len(),
            beats[0].time_seconds,
            beats.last().unwrap().time_seconds,
            beats[0].bpm
        );
    }

    fn synth_anlz_no_pqtz() -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(b"PMAI");
        buf.extend_from_slice(&28u32.to_be_bytes());
        let len_file_off = buf.len();
        buf.extend_from_slice(&0u32.to_be_bytes());
        for _ in 0..4 {
            buf.extend_from_slice(&0u32.to_be_bytes());
        }
        buf.extend_from_slice(b"PPTH");
        buf.extend_from_slice(&16u32.to_be_bytes());
        let total = TAG_ENVELOPE_LEN + 4;
        buf.extend_from_slice(&(total as u32).to_be_bytes());
        buf.extend_from_slice(&0u32.to_be_bytes());
        let len_file = buf.len() as u32;
        buf[len_file_off..len_file_off + 4].copy_from_slice(&len_file.to_be_bytes());
        buf
    }
}
