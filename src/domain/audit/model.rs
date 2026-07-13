use std::collections::HashMap;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, strum::EnumString, strum::EnumIter, strum::Display,
)]
pub enum IssueType {
    #[strum(serialize = "EMPTY_ARTIST")]
    EmptyArtist,
    #[strum(serialize = "EMPTY_TITLE")]
    EmptyTitle,
    #[strum(serialize = "MISSING_TRACK_NUM")]
    MissingTrackNum,
    #[strum(serialize = "MISSING_ALBUM")]
    MissingAlbum,
    #[strum(serialize = "MISSING_YEAR")]
    MissingYear,
    #[strum(serialize = "ARTIST_IN_TITLE")]
    ArtistInTitle,
    #[strum(serialize = "WAV_TAG3_MISSING")]
    WavTag3Missing,
    #[strum(serialize = "WAV_TAG_DRIFT")]
    WavTagDrift,
    #[strum(serialize = "GENRE_SET")]
    GenreSet,
    #[strum(serialize = "NO_TAGS")]
    NoTags,
    #[strum(serialize = "BAD_FILENAME")]
    BadFilename,
    #[strum(serialize = "ORIGINAL_MIX_SUFFIX")]
    OriginalMixSuffix,
    #[strum(serialize = "TECH_SPECS_IN_DIR")]
    TechSpecsInDir,
    #[strum(serialize = "MISSING_YEAR_IN_DIR")]
    MissingYearInDir,
    #[strum(serialize = "FILENAME_TAG_DRIFT")]
    FilenameTagDrift,
}

impl IssueType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::EmptyArtist => "EMPTY_ARTIST",
            Self::EmptyTitle => "EMPTY_TITLE",
            Self::MissingTrackNum => "MISSING_TRACK_NUM",
            Self::MissingAlbum => "MISSING_ALBUM",
            Self::MissingYear => "MISSING_YEAR",
            Self::ArtistInTitle => "ARTIST_IN_TITLE",
            Self::WavTag3Missing => "WAV_TAG3_MISSING",
            Self::WavTagDrift => "WAV_TAG_DRIFT",
            Self::GenreSet => "GENRE_SET",
            Self::NoTags => "NO_TAGS",
            Self::BadFilename => "BAD_FILENAME",
            Self::OriginalMixSuffix => "ORIGINAL_MIX_SUFFIX",
            Self::TechSpecsInDir => "TECH_SPECS_IN_DIR",
            Self::MissingYearInDir => "MISSING_YEAR_IN_DIR",
            Self::FilenameTagDrift => "FILENAME_TAG_DRIFT",
        }
    }

    #[cfg(test)]
    pub fn safety_tier(&self) -> SafetyTier {
        match self {
            Self::ArtistInTitle | Self::WavTag3Missing | Self::WavTagDrift => SafetyTier::Safe,
            Self::OriginalMixSuffix | Self::TechSpecsInDir => SafetyTier::RenameSafe,
            Self::EmptyArtist
            | Self::EmptyTitle
            | Self::MissingTrackNum
            | Self::MissingAlbum
            | Self::MissingYear
            | Self::GenreSet
            | Self::NoTags
            | Self::BadFilename
            | Self::MissingYearInDir
            | Self::FilenameTagDrift => SafetyTier::Review,
        }
    }
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SafetyTier {
    Safe,
    RenameSafe,
    Review,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AuditStatus {
    Open,
    Resolved,
    Accepted,
    Deferred,
}

impl AuditStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Resolved => "resolved",
            Self::Accepted => "accepted",
            Self::Deferred => "deferred",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "open" => Some(Self::Open),
            "resolved" => Some(Self::Resolved),
            "accepted" => Some(Self::Accepted),
            "deferred" => Some(Self::Deferred),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Resolution {
    AcceptedAsIs,
    WontFix,
    Deferred,
    Fixed,
}

impl Resolution {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AcceptedAsIs => "accepted_as_is",
            Self::WontFix => "wont_fix",
            Self::Deferred => "deferred",
            Self::Fixed => "fixed",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "accepted_as_is" => Some(Self::AcceptedAsIs),
            "wont_fix" => Some(Self::WontFix),
            "deferred" => Some(Self::Deferred),
            "fixed" => Some(Self::Fixed),
            _ => None,
        }
    }

    pub fn status(&self) -> AuditStatus {
        match self {
            Self::AcceptedAsIs | Self::WontFix => AuditStatus::Accepted,
            Self::Deferred => AuditStatus::Deferred,
            Self::Fixed => AuditStatus::Resolved,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditContext {
    AlbumTrack,
    LooseTrack,
}

/// Transport-neutral projection of the tag data needed by audit policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TagSnapshot {
    Single {
        tags: HashMap<String, Option<String>>,
    },
    Wav {
        id3v2: HashMap<String, Option<String>>,
        riff_info: HashMap<String, Option<String>>,
        tag3_missing: Vec<String>,
    },
    Error,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_resolution_round_trips_through_its_wire_value() {
        for resolution in [
            Resolution::AcceptedAsIs,
            Resolution::WontFix,
            Resolution::Deferred,
            Resolution::Fixed,
        ] {
            assert_eq!(Resolution::from_str(resolution.as_str()), Some(resolution));
        }
        assert_eq!(Resolution::from_str("accepted-as-is"), None);
    }
}
