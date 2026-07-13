//! Staged metadata records and policies.
//!
//! Metadata domain code must remain independent of CLI, MCP, database, store,
//! XML, and tag-adapter infrastructure.

pub(crate) mod changes;
mod color;
mod model;
mod normalization;

pub(crate) use changes::ChangeManager;
#[cfg(test)]
pub(crate) use color::color_name_to_code;
pub(crate) use color::{COLORS, canonical_color_name, is_valid_color};
pub(crate) use model::{EditableField, TrackChange, TrackDiff};
pub(crate) use normalization::normalize_for_matching;
