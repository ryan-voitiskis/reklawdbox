use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Audio file format as identified by Rekordbox's integer file-type code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileKind {
    Mp3,
    M4a,
    Flac,
    Wav,
    Aiff,
    Unknown(i32),
}

impl FileKind {
    pub fn from_raw(raw: i32) -> Self {
        match raw {
            1 => Self::Mp3,
            4 => Self::M4a,
            5 => Self::Flac,
            11 => Self::Wav,
            12 => Self::Aiff,
            _ => Self::Unknown(raw),
        }
    }

    /// Matches Rekordbox XML `Kind` attribute values.
    pub fn as_kind_str(&self) -> &'static str {
        match self {
            Self::Mp3 => "MP3 File",
            Self::M4a => "M4A File",
            Self::Flac => "FLAC File",
            Self::Wav => "WAV File",
            Self::Aiff => "AIFF File",
            Self::Unknown(_) => "Audio File",
        }
    }
}

impl Serialize for FileKind {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_kind_str())
    }
}

impl<'de> Deserialize<'de> for FileKind {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Ok(match s.as_str() {
            "MP3 File" => Self::Mp3,
            "M4A File" => Self::M4a,
            "FLAC File" => Self::Flac,
            "WAV File" => Self::Wav,
            "AIFF File" => Self::Aiff,
            _ => Self::Unknown(0),
        })
    }
}

impl JsonSchema for FileKind {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed("FileKind")
    }

    fn json_schema(_gen: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "enum": ["MP3 File", "M4A File", "FLAC File", "WAV File", "AIFF File", "Audio File"]
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Track {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub genre: String,
    pub bpm: f64,
    pub key: String,
    pub rating: u8, // 0-5 stars
    pub comments: String,
    pub color: String,
    pub color_code: i32,
    pub label: String,
    pub remixer: String,
    pub year: i32,
    pub length: i32,       // seconds
    pub file_path: String, // DB FolderPath
    pub play_count: i32,
    pub bit_rate: i32,
    pub sample_rate: i32,
    #[serde(rename = "file_type_name")]
    pub file_kind: FileKind,
    pub date_added: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<u32>,
    /// Timestamp when the track was loaded in a DJ session (only set in session track lists).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub played_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Playlist {
    pub id: String,
    pub name: String,
    pub track_count: i32,
    pub parent_id: String,
    pub is_folder: bool,
    pub is_smart: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Session {
    pub id: String,
    pub name: String,
    pub date_created: String,
    pub track_count: i32,
    /// Wall-clock estimate in seconds, or null if < 2 tracks
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TrackPlayStats {
    pub track_id: String,
    pub title: String,
    pub artist: String,
    pub play_count: i32,
    pub session_count: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_played: Option<String>,
    pub session_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LibraryStats {
    pub total_tracks: i32,
    pub genres: Vec<GenreCount>,
    pub playlist_count: i32,
    pub rated_count: i32,
    pub unrated_count: i32,
    pub avg_bpm: f64,
    pub key_distribution: Vec<KeyCount>,
    pub content_roots: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GenreCount {
    pub name: String,
    pub count: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct KeyCount {
    pub name: String,
    pub count: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file_kind_to_raw(kind: FileKind) -> i32 {
        match kind {
            FileKind::Mp3 => 1,
            FileKind::M4a => 4,
            FileKind::Flac => 5,
            FileKind::Wav => 11,
            FileKind::Aiff => 12,
            FileKind::Unknown(raw) => raw,
        }
    }

    #[test]
    fn file_kind_raw_roundtrip() {
        for kind in [
            FileKind::Mp3,
            FileKind::M4a,
            FileKind::Flac,
            FileKind::Wav,
            FileKind::Aiff,
        ] {
            assert_eq!(
                FileKind::from_raw(file_kind_to_raw(kind)),
                kind,
                "roundtrip failed for {kind:?}"
            );
        }
    }

    #[test]
    fn file_kind_unknown_preserves_raw() {
        let kind = FileKind::Unknown(99);
        assert_eq!(file_kind_to_raw(kind), 99);
        assert_eq!(kind.as_kind_str(), "Audio File");
    }

    #[test]
    fn file_kind_serializes_as_kind_str() {
        let json = serde_json::to_value(FileKind::Flac).unwrap();
        assert_eq!(json, serde_json::Value::String("FLAC File".to_string()));

        let json = serde_json::to_value(FileKind::Unknown(42)).unwrap();
        assert_eq!(json, serde_json::Value::String("Audio File".to_string()));
    }

    #[test]
    fn file_kind_deserializes_from_kind_str() {
        let kind: FileKind = serde_json::from_value(serde_json::json!("FLAC File")).unwrap();
        assert_eq!(kind, FileKind::Flac);

        let kind: FileKind = serde_json::from_value(serde_json::json!("MP3 File")).unwrap();
        assert_eq!(kind, FileKind::Mp3);

        let kind: FileKind = serde_json::from_value(serde_json::json!("Ogg File")).unwrap();
        assert_eq!(kind, FileKind::Unknown(0));
    }

    #[test]
    fn track_serde_roundtrip_preserves_wire_fields() {
        let track = Track {
            id: "track-1".into(),
            title: "Title".into(),
            artist: "Artist".into(),
            album: "Album".into(),
            genre: "Techno".into(),
            bpm: 128.0,
            key: "Am".into(),
            rating: 4,
            comments: "Comment".into(),
            color: "Blue".into(),
            color_code: 0x0000FF,
            label: "Label".into(),
            remixer: "Remixer".into(),
            year: 2026,
            length: 360,
            file_path: "/music/track.flac".into(),
            play_count: 2,
            bit_rate: 1411,
            sample_rate: 44100,
            file_kind: FileKind::Flac,
            date_added: "2026-07-14".into(),
            position: Some(7),
            played_at: Some("2026-07-14T12:00:00Z".into()),
        };

        let value = serde_json::to_value(&track).unwrap();
        assert_eq!(value["file_type_name"], "FLAC File");
        assert_eq!(value["position"], 7);
        assert_eq!(value["played_at"], "2026-07-14T12:00:00Z");
        let decoded: Track = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), value);
    }
}
