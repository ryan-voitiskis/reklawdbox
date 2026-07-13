use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct TrackChange {
    pub track_id: String,
    pub genre: Option<String>,
    pub comments: Option<String>,
    pub rating: Option<u8>, // 1-5 stars
    pub color: Option<String>,
    pub label: Option<String>,
    pub year: Option<i32>,
    pub album: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EditableField {
    Genre,
    Comments,
    Rating,
    Color,
    Label,
    Year,
    Album,
}

impl EditableField {
    pub const ALL: &[Self] = &[
        Self::Genre,
        Self::Comments,
        Self::Rating,
        Self::Color,
        Self::Label,
        Self::Year,
        Self::Album,
    ];

    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Genre => "genre",
            Self::Comments => "comments",
            Self::Rating => "rating",
            Self::Color => "color",
            Self::Label => "label",
            Self::Year => "year",
            Self::Album => "album",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "genre" => Some(Self::Genre),
            "comments" => Some(Self::Comments),
            "rating" => Some(Self::Rating),
            "color" => Some(Self::Color),
            "label" => Some(Self::Label),
            "year" => Some(Self::Year),
            "album" => Some(Self::Album),
            _ => None,
        }
    }

    pub fn all_names_csv() -> String {
        Self::ALL
            .iter()
            .map(EditableField::as_str)
            .collect::<Vec<_>>()
            .join(", ")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editable_field_count_matches_track_change() {
        let json = serde_json::to_value(TrackChange {
            track_id: "x".into(),
            genre: None,
            comments: None,
            rating: None,
            color: None,
            label: None,
            year: None,
            album: None,
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

    #[test]
    fn track_change_serde_roundtrip_preserves_optional_fields() {
        let change = TrackChange {
            track_id: "track-1".into(),
            genre: Some("Techno".into()),
            comments: None,
            rating: Some(5),
            color: Some("Blue".into()),
            label: None,
            year: Some(2026),
            album: Some("Album".into()),
        };

        let value = serde_json::to_value(&change).unwrap();
        assert!(value["comments"].is_null());
        assert_eq!(value["rating"], 5);
        let decoded: TrackChange = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), value);
    }
}
