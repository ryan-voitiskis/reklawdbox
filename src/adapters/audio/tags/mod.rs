//! Audio-file tag reading and atomic mutation adapter using `lofty`.
//!
//! Pure synchronous capabilities shared by CLI and MCP application workflows.

mod art;
mod fields;
mod model;
mod mutation;
mod read;

#[cfg(test)]
pub(crate) use art::ACCEPTED_PICTURE_TYPES;
pub(crate) use art::{embed_cover_art, extract_cover_art, parse_picture_type};
pub(crate) use fields::ALL_FIELDS;
pub(crate) use model::{
    CommentMode, DryRunChange, ExtractArtResult, FileDryRunResult, FileEmbedResult, FileReadResult,
    FileWriteResult, WavTarget, WriteEntry,
};
pub(crate) use mutation::{write_file_tags, write_file_tags_dry_run};
pub(crate) use read::read_file_tags;

#[cfg(test)]
mod tests;
