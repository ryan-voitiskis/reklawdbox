use std::fmt;

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
    /// Convert from Rekordbox integer file type code.
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

    /// Rekordbox integer file type code (for XML export).
    pub fn to_raw(self) -> i32 {
        match self {
            Self::Mp3 => 1,
            Self::M4a => 4,
            Self::Flac => 5,
            Self::Wav => 11,
            Self::Aiff => 12,
            Self::Unknown(raw) => raw,
        }
    }

    /// Human-readable kind string matching Rekordbox XML `Kind` attribute.
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
pub struct TrackChange {
    pub track_id: String,
    pub genre: Option<String>,
    pub comments: Option<String>,
    pub rating: Option<u8>, // 1-5 stars
    pub color: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EditableField {
    Genre,
    Comments,
    Rating,
    Color,
}

impl EditableField {
    pub const ALL: &[Self] = &[Self::Genre, Self::Comments, Self::Rating, Self::Color];

    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Genre => "genre",
            Self::Comments => "comments",
            Self::Rating => "rating",
            Self::Color => "color",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "genre" => Some(Self::Genre),
            "comments" => Some(Self::Comments),
            "rating" => Some(Self::Rating),
            "color" => Some(Self::Color),
            _ => None,
        }
    }

    /// Comma-separated list of all field names (for error messages and descriptions).
    pub fn all_names_csv() -> String {
        Self::ALL.iter().map(|f| f.as_str()).collect::<Vec<_>>().join(", ")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FieldDiff {
    pub field: String,
    pub old_value: String,
    pub new_value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TrackDiff {
    pub track_id: String,
    pub title: String,
    pub artist: String,
    pub changes: Vec<FieldDiff>,
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

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NormalizationSuggestion {
    pub track_id: String,
    pub title: String,
    pub artist: String,
    pub current_genre: String,
    pub suggested_genre: Option<String>,
    pub confidence: Confidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    Alias,
    Unknown,
    Canonical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Discogs,
    Beatport,
}

impl Provider {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Discogs => "discogs",
            Self::Beatport => "beatport",
        }
    }
}

impl fmt::Display for Provider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Convert 0-5 star rating to Rekordbox DB/XML encoding (0/51/102/153/204/255).
pub fn stars_to_rating(stars: u8) -> u16 {
    match stars {
        0 => 0,
        1 => 51,
        2 => 102,
        3 => 153,
        4 => 204,
        5 => 255,
        _ => 255,
    }
}

/// Convert Rekordbox DB/XML rating encoding to 0-5 stars.
pub fn rating_to_stars(rating: u16) -> u8 {
    match rating {
        0..=25 => 0,
        26..=76 => 1,
        77..=127 => 2,
        128..=178 => 3,
        179..=229 => 4,
        230..=255 => 5,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rating_roundtrip() {
        for stars in 0..=5u8 {
            let encoded = stars_to_rating(stars);
            let decoded = rating_to_stars(encoded);
            assert_eq!(
                stars, decoded,
                "roundtrip failed for {stars} stars (encoded: {encoded})"
            );
        }
    }

    #[test]
    fn rating_exact_values() {
        assert_eq!(stars_to_rating(0), 0);
        assert_eq!(stars_to_rating(1), 51);
        assert_eq!(stars_to_rating(2), 102);
        assert_eq!(stars_to_rating(3), 153);
        assert_eq!(stars_to_rating(4), 204);
        assert_eq!(stars_to_rating(5), 255);
    }

    #[test]
    fn stars_out_of_range_saturates_to_five_stars() {
        assert_eq!(stars_to_rating(6), 255);
        assert_eq!(stars_to_rating(u8::MAX), 255);
    }

    #[test]
    fn rating_bucket_boundaries() {
        assert_eq!(rating_to_stars(25), 0);
        assert_eq!(rating_to_stars(26), 1);
        assert_eq!(rating_to_stars(76), 1);
        assert_eq!(rating_to_stars(77), 2);
        assert_eq!(rating_to_stars(127), 2);
        assert_eq!(rating_to_stars(128), 3);
        assert_eq!(rating_to_stars(178), 3);
        assert_eq!(rating_to_stars(179), 4);
        assert_eq!(rating_to_stars(229), 4);
        assert_eq!(rating_to_stars(230), 5);
        assert_eq!(rating_to_stars(255), 5);
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
                FileKind::from_raw(kind.to_raw()),
                kind,
                "roundtrip failed for {kind:?}"
            );
        }
    }

    #[test]
    fn file_kind_unknown_preserves_raw() {
        let kind = FileKind::Unknown(99);
        assert_eq!(kind.to_raw(), 99);
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
    fn editable_field_count_matches_track_change() {
        let json = serde_json::to_value(TrackChange {
            track_id: "x".into(),
            genre: None,
            comments: None,
            rating: None,
            color: None,
        })
        .unwrap();
        let field_count = json.as_object().unwrap().len() - 1; // minus track_id
        assert_eq!(
            field_count,
            EditableField::ALL.len(),
            "TrackChange has {field_count} editable fields but EditableField has {} variants. \
             Update EditableField when adding fields.",
            EditableField::ALL.len(),
        );
    }
}
