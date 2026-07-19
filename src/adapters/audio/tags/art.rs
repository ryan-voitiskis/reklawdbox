//! Cover-art extraction and embedding I/O.

use std::fs;
use std::path::Path;

use lofty::config::WriteOptions;
use lofty::file::{FileType, TaggedFileExt};
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::prelude::*;
use lofty::probe::Probe;
use lofty::tag::{Tag, TagType};

use super::model::{ExtractArtResult, FileEmbedResult, TagError};
use super::read::parse_options;

/// Exact, case-sensitive picture type names accepted by cover-art operations.
pub(crate) const ACCEPTED_PICTURE_TYPES: &[&str] = &[
    "other",
    "icon",
    "other_icon",
    "front_cover",
    "cover_front",
    "back_cover",
    "cover_back",
    "leaflet",
    "media",
    "lead_artist",
    "artist",
    "conductor",
    "band",
    "composer",
    "lyricist",
    "recording_location",
    "during_recording",
    "during_performance",
    "screen_capture",
    "bright_fish",
    "illustration",
    "band_logo",
    "publisher_logo",
];

pub fn parse_picture_type(name: &str) -> Result<PictureType, TagError> {
    let picture_type = match name {
        "other" => PictureType::Other,
        "icon" => PictureType::Icon,
        "other_icon" => PictureType::OtherIcon,
        "front_cover" | "cover_front" => PictureType::CoverFront,
        "back_cover" | "cover_back" => PictureType::CoverBack,
        "leaflet" => PictureType::Leaflet,
        "media" => PictureType::Media,
        "lead_artist" => PictureType::LeadArtist,
        "artist" => PictureType::Artist,
        "conductor" => PictureType::Conductor,
        "band" => PictureType::Band,
        "composer" => PictureType::Composer,
        "lyricist" => PictureType::Lyricist,
        "recording_location" => PictureType::RecordingLocation,
        "during_recording" => PictureType::DuringRecording,
        "during_performance" => PictureType::DuringPerformance,
        "screen_capture" => PictureType::ScreenCapture,
        "bright_fish" => PictureType::BrightFish,
        "illustration" => PictureType::Illustration,
        "band_logo" => PictureType::BandLogo,
        "publisher_logo" => PictureType::PublisherLogo,
        _ => {
            return Err(TagError::Validation(format!(
                "Unknown picture type {name:?}. Accepted values: {}",
                ACCEPTED_PICTURE_TYPES.join(", ")
            )));
        }
    };
    Ok(picture_type)
}

pub(super) fn picture_type_name(pt: PictureType) -> &'static str {
    match pt {
        PictureType::Other => "other",
        PictureType::Icon => "icon",
        PictureType::OtherIcon => "other_icon",
        PictureType::CoverFront => "front_cover",
        PictureType::CoverBack => "back_cover",
        PictureType::Leaflet => "leaflet",
        PictureType::Media => "media",
        PictureType::LeadArtist => "lead_artist",
        PictureType::Artist => "artist",
        PictureType::Conductor => "conductor",
        PictureType::Band => "band",
        PictureType::Composer => "composer",
        PictureType::Lyricist => "lyricist",
        PictureType::RecordingLocation => "recording_location",
        PictureType::DuringRecording => "during_recording",
        PictureType::DuringPerformance => "during_performance",
        PictureType::ScreenCapture => "screen_capture",
        PictureType::BrightFish => "bright_fish",
        PictureType::Illustration => "illustration",
        PictureType::BandLogo => "band_logo",
        PictureType::PublisherLogo => "publisher_logo",
        _ => "other",
    }
}

fn mime_extension(mime: Option<&MimeType>) -> &'static str {
    match mime {
        Some(MimeType::Jpeg) => "jpg",
        Some(MimeType::Png) => "png",
        Some(MimeType::Tiff) => "tif",
        Some(MimeType::Bmp) => "bmp",
        Some(MimeType::Gif) => "gif",
        _ => "bin",
    }
}

fn mime_name(mime: Option<&MimeType>) -> &'static str {
    match mime {
        Some(MimeType::Jpeg) => "jpeg",
        Some(MimeType::Png) => "png",
        Some(MimeType::Tiff) => "tiff",
        Some(MimeType::Bmp) => "bmp",
        Some(MimeType::Gif) => "gif",
        _ => "unknown",
    }
}

// ---------------------------------------------------------------------------
// 4. extract_cover_art
// ---------------------------------------------------------------------------

/// Extract embedded cover art to disk.
///
/// For WAV files, reads from ID3v2 only (RIFF INFO does not support images).
/// If `output_path` is `None`, writes to `{parent_dir}/cover.{ext}`.
pub fn extract_cover_art(
    path: &Path,
    output_path: Option<&Path>,
    picture_type: &str,
) -> Result<ExtractArtResult, TagError> {
    let path_str = path.display().to_string();
    let pic_type = parse_picture_type(picture_type)?;

    let tagged_file = Probe::open(path)
        .map_err(|e| TagError::Io(format!("Failed to open: {e}")))?
        .options(parse_options(true))
        .read()
        .map_err(|e| TagError::Io(format!("Failed to read: {e}")))?;

    let file_type = tagged_file.file_type();

    let tag = if file_type == FileType::Wav {
        tagged_file.tag(TagType::Id3v2)
    } else {
        tagged_file
            .primary_tag()
            .or_else(|| tagged_file.first_tag())
    };

    let tag = tag.ok_or(TagError::NoTags)?;

    let picture = tag
        .pictures()
        .iter()
        .find(|p| p.pic_type() == pic_type)
        .or_else(|| tag.pictures().first())
        .ok_or(TagError::NoPicture)?;

    let ext = mime_extension(picture.mime_type());
    let image_format = mime_name(picture.mime_type());

    let out_path = match output_path {
        Some(p) => p.to_path_buf(),
        None => {
            let parent = path.parent().unwrap_or(Path::new("."));
            parent.join(format!("cover.{ext}"))
        }
    };

    fs::write(&out_path, picture.data())
        .map_err(|e| TagError::Io(format!("Failed to write cover art: {e}")))?;

    Ok(ExtractArtResult {
        path: path_str,
        output_path: out_path.display().to_string(),
        image_format: image_format.to_string(),
        size_bytes: picture.data().len(),
        picture_type: picture_type_name(picture.pic_type()).to_string(),
    })
}

// ---------------------------------------------------------------------------
// 5. embed_cover_art
// ---------------------------------------------------------------------------

/// Embed an image file into an audio file as cover art.
///
/// For WAV files, writes to ID3v2 only (RIFF INFO does not support images).
pub fn embed_cover_art(
    image_path: &Path,
    target_path: &Path,
    picture_type: &str,
) -> FileEmbedResult {
    let target_str = target_path.display().to_string();

    match embed_cover_art_inner(image_path, target_path, picture_type) {
        Ok(()) => FileEmbedResult::Ok {
            path: target_str,
            status: "ok".to_string(),
        },
        Err(e) => FileEmbedResult::Error {
            path: target_str,
            status: "error".to_string(),
            error: e.to_string(),
        },
    }
}

pub(super) fn embed_cover_art_inner(
    image_path: &Path,
    target_path: &Path,
    picture_type_str: &str,
) -> Result<(), TagError> {
    let pic_type = parse_picture_type(picture_type_str)?;

    let image_data =
        fs::read(image_path).map_err(|e| TagError::Io(format!("Failed to read image: {e}")))?;

    // Detect MIME type from the data, then drop the temporary Picture to free its copy
    let mime = {
        let mut cursor = std::io::Cursor::new(&image_data);
        let detected = Picture::from_reader(&mut cursor)
            .map_err(|e| TagError::Io(format!("Failed to parse image: {e}")))?;
        detected.mime_type().cloned()
    };

    let mut builder = Picture::unchecked(image_data).pic_type(pic_type);
    if let Some(mime) = mime {
        builder = builder.mime_type(mime);
    }
    let picture = builder.build();

    let mut tagged_file = Probe::open(target_path)
        .map_err(|e| TagError::Io(format!("Failed to open: {e}")))?
        .options(parse_options(true))
        .read()
        .map_err(|e| TagError::Io(format!("Failed to read: {e}")))?;

    let file_type = tagged_file.file_type();

    if file_type == FileType::Wav {
        let tag = match tagged_file.tag_mut(TagType::Id3v2) {
            Some(t) => t,
            None => {
                tagged_file.insert_tag(Tag::new(TagType::Id3v2));
                tagged_file
                    .tag_mut(TagType::Id3v2)
                    .ok_or(TagError::Unsupported(
                        "WAV file does not support ID3v2".to_string(),
                    ))?
            }
        };

        tag.remove_picture_type(pic_type);
        tag.push_picture(picture);

        tag.save_to_path(target_path, WriteOptions::default())
            .map_err(|e| TagError::Io(format!("Failed to write ID3v2 tag: {e}")))?;
    } else {
        // Single tag layer — use primary tag type
        let primary_type = file_type.primary_tag_type();
        let tag = match tagged_file.tag_mut(primary_type) {
            Some(t) => t,
            None => {
                tagged_file.insert_tag(Tag::new(primary_type));
                tagged_file.tag_mut(primary_type).ok_or_else(|| {
                    TagError::Unsupported(format!("File does not support {primary_type:?} tags"))
                })?
            }
        };

        tag.remove_picture_type(pic_type);
        tag.push_picture(picture);

        tag.save_to_path(target_path, WriteOptions::default())
            .map_err(|e| TagError::Io(format!("Failed to write tag: {e}")))?;
    }

    Ok(())
}
