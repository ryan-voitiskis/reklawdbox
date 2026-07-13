//! Staged metadata records and policies.
//!
//! Metadata domain code must remain independent of CLI, MCP, database, store,
//! XML, and tag-adapter infrastructure.

pub(crate) mod changes;
mod color;
mod model;

pub(crate) use changes::{ChangeManager, ChangeSnapshotGuard};
pub(crate) use color::{COLORS, canonical_color_name, color_name_to_code, is_valid_color};
pub(crate) use model::{EditableField, FieldDiff, TrackChange, TrackDiff};
