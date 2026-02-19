use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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
    pub file_type: i32,
    pub date_added: String,
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

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ChangeDiff {
    pub track_id: String,
    pub title: String,
    pub artist: String,
    pub field: String,
    pub old_value: String,
    pub new_value: String,
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
    pub confidence: String, // "alias" | "unknown" | "canonical"
}

/// Convert 1-5 star rating to Rekordbox DB/XML encoding (0/51/102/153/204/255).
pub fn stars_to_rating(stars: u8) -> u16 {
    match stars {
        0 => 0,
        1 => 51,
        2 => 102,
        3 => 153,
        4 => 204,
        5 => 255,
        _ => 0,
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
}
