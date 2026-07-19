//! Canonical audio-tag fields, validation, mapping, and comment policy.

use std::collections::{BTreeMap, HashMap};

use lofty::tag::{ItemKey, Tag};

use super::model::{CommentMode, TagError};

/// All 14 canonical field names, in a stable order.
pub const ALL_FIELDS: &[&str] = &[
    "artist",
    "title",
    "album",
    "album_artist",
    "genre",
    "year",
    "track",
    "disc",
    "comment",
    "publisher",
    "bpm",
    "key",
    "composer",
    "remixer",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum TagField {
    Artist,
    Title,
    Album,
    AlbumArtist,
    Genre,
    Year,
    Track,
    Disc,
    Comment,
    Publisher,
    Bpm,
    Key,
    Composer,
    Remixer,
}

impl TagField {
    #[cfg(test)]
    pub(super) const ALL: [Self; 14] = [
        Self::Artist,
        Self::Title,
        Self::Album,
        Self::AlbumArtist,
        Self::Genre,
        Self::Year,
        Self::Track,
        Self::Disc,
        Self::Comment,
        Self::Publisher,
        Self::Bpm,
        Self::Key,
        Self::Composer,
        Self::Remixer,
    ];

    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::Artist => "artist",
            Self::Title => "title",
            Self::Album => "album",
            Self::AlbumArtist => "album_artist",
            Self::Genre => "genre",
            Self::Year => "year",
            Self::Track => "track",
            Self::Disc => "disc",
            Self::Comment => "comment",
            Self::Publisher => "publisher",
            Self::Bpm => "bpm",
            Self::Key => "key",
            Self::Composer => "composer",
            Self::Remixer => "remixer",
        }
    }

    pub(super) fn parse(value: &str) -> Option<Self> {
        match value {
            "artist" => Some(Self::Artist),
            "title" => Some(Self::Title),
            "album" => Some(Self::Album),
            "album_artist" => Some(Self::AlbumArtist),
            "genre" => Some(Self::Genre),
            "year" => Some(Self::Year),
            "track" => Some(Self::Track),
            "disc" => Some(Self::Disc),
            "comment" => Some(Self::Comment),
            "publisher" => Some(Self::Publisher),
            "bpm" => Some(Self::Bpm),
            "key" => Some(Self::Key),
            "composer" => Some(Self::Composer),
            "remixer" => Some(Self::Remixer),
            _ => None,
        }
    }

    pub(super) const fn is_riff_info(self) -> bool {
        matches!(
            self,
            Self::Artist | Self::Title | Self::Album | Self::Genre | Self::Year | Self::Comment
        )
    }

    pub(super) const fn primary_item_key(self) -> ItemKey {
        match self {
            Self::Artist => ItemKey::TrackArtist,
            Self::Title => ItemKey::TrackTitle,
            Self::Album => ItemKey::AlbumTitle,
            Self::AlbumArtist => ItemKey::AlbumArtist,
            Self::Genre => ItemKey::Genre,
            Self::Year => ItemKey::RecordingDate,
            Self::Track => ItemKey::TrackNumber,
            Self::Disc => ItemKey::DiscNumber,
            Self::Comment => ItemKey::Comment,
            Self::Publisher => ItemKey::Label,
            Self::Bpm => ItemKey::IntegerBpm,
            Self::Key => ItemKey::InitialKey,
            Self::Composer => ItemKey::Composer,
            Self::Remixer => ItemKey::Remixer,
        }
    }

    pub(super) const fn secondary_item_key(self) -> Option<ItemKey> {
        match self {
            Self::Year => Some(ItemKey::Year),
            Self::Bpm => Some(ItemKey::Bpm),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum TagEdit {
    Set(String),
    Delete,
}

#[derive(Debug)]
pub(super) struct ValidatedTagPatch {
    pub(super) edits: BTreeMap<TagField, TagEdit>,
}

impl TryFrom<&HashMap<String, Option<String>>> for ValidatedTagPatch {
    type Error = TagError;

    fn try_from(tags: &HashMap<String, Option<String>>) -> Result<Self, Self::Error> {
        let mut edits = BTreeMap::new();

        for (raw_field, raw_value) in tags {
            let field = TagField::parse(raw_field)
                .ok_or_else(|| TagError::Validation(format!("Unknown field \"{raw_field}\"")))?;
            let edit = match raw_value {
                None => TagEdit::Delete,
                Some(value) if value.is_empty() => TagEdit::Delete,
                Some(value) => {
                    match field {
                        TagField::Year => {
                            if value.len() != 4 || value.parse::<u16>().is_err() {
                                return Err(TagError::Validation(format!(
                                    "Invalid year \"{value}\": must be 4-digit YYYY or null/empty to delete"
                                )));
                            }
                        }
                        TagField::Track | TagField::Disc => match value.parse::<u32>() {
                            Ok(number) if number > 0 => {}
                            _ => {
                                return Err(TagError::Validation(format!(
                                    "Invalid {} \"{value}\": must be a positive integer or null/empty to delete",
                                    field.as_str()
                                )));
                            }
                        },
                        _ => {}
                    }
                    TagEdit::Set(value.clone())
                }
            };
            edits.insert(field, edit);
        }

        Ok(Self { edits })
    }
}

/// Reverse mapping from `ItemKey` to canonical field name (test-only).
#[cfg(test)]
pub(super) fn item_key_to_field(key: &ItemKey) -> Option<&'static str> {
    match *key {
        ItemKey::TrackArtist => Some("artist"),
        ItemKey::TrackTitle => Some("title"),
        ItemKey::AlbumTitle => Some("album"),
        ItemKey::AlbumArtist => Some("album_artist"),
        ItemKey::Genre => Some("genre"),
        ItemKey::RecordingDate => Some("year"),
        ItemKey::Year => Some("year"),
        ItemKey::TrackNumber => Some("track"),
        ItemKey::DiscNumber => Some("disc"),
        ItemKey::Comment => Some("comment"),
        ItemKey::Label => Some("publisher"),
        ItemKey::IntegerBpm => Some("bpm"),
        ItemKey::Bpm => Some("bpm"),
        ItemKey::InitialKey => Some("key"),
        ItemKey::Composer => Some("composer"),
        ItemKey::Remixer => Some("remixer"),
        _ => None,
    }
}

pub(super) fn is_riff_info_field(field: &str) -> bool {
    TagField::parse(field).is_some_and(TagField::is_riff_info)
}

const COMMENT_SEPARATOR: &str = " | ";

pub(super) fn merge_comment(new: &str, existing: Option<&str>, mode: CommentMode) -> String {
    match mode {
        CommentMode::Replace => new.to_string(),
        CommentMode::Prepend => match existing {
            Some(ex) if !ex.is_empty() => format!("{new}{COMMENT_SEPARATOR}{ex}"),
            _ => new.to_string(),
        },
        CommentMode::Append => match existing {
            Some(ex) if !ex.is_empty() => format!("{ex}{COMMENT_SEPARATOR}{new}"),
            _ => new.to_string(),
        },
    }
}

pub(super) fn get_tag_field(tag: &Tag, field: TagField) -> Option<String> {
    let primary = field.primary_item_key();

    if let Some(val) = tag.get_string(primary) {
        return Some(val.to_string());
    }

    match field {
        TagField::Year => tag
            .get_string(ItemKey::Year)
            .map(std::string::ToString::to_string),
        TagField::Bpm => tag
            .get_string(ItemKey::Bpm)
            .map(std::string::ToString::to_string),
        _ => None,
    }
}

pub(super) fn get_field_from_tag(tag: &Tag, field: &str) -> Option<String> {
    get_tag_field(tag, TagField::parse(field)?)
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Validate tag values before writing.
///
/// Rules:
/// - `year`: must be 4-digit YYYY or null/empty (delete)
/// - `track`, `disc`: must be positive integer or null/empty (delete)
/// - All other fields: accepted as-is
#[cfg(test)]
pub(super) fn validate_write_tags(tags: &HashMap<String, Option<String>>) -> Result<(), TagError> {
    ValidatedTagPatch::try_from(tags).map(|_| ())
}
