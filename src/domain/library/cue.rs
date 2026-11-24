use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CueKind {
    Memory,
    Hot,
    Unknown,
}

impl CueKind {
    /// Slots A–H have a confirmed mapping. Preserve every other raw kind.
    pub(crate) fn from_raw(raw: Option<i64>) -> (Self, Option<char>) {
        match raw {
            Some(0) => (Self::Memory, None),
            Some(slot @ 1..=8) => (Self::Hot, Some((b'A' + slot as u8 - 1) as char)),
            _ => (Self::Unknown, None),
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct Cue {
    pub cue_id: String,
    pub track_id: String,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub kind: CueKind,
    pub raw_kind: Option<i64>,
    pub hot_slot: Option<char>,
    pub start_ms: Option<i64>,
    pub loop_end_ms: Option<i64>,
    pub comment: Option<String>,
    pub color: Option<i64>,
    pub color_table_index: Option<i64>,
    pub active_loop: Option<i64>,
}
