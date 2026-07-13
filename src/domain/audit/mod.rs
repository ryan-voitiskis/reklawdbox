//! Neutral audit vocabulary shared by audit logic and persistence adapters.
//!
//! Audit domain code must not depend on application infrastructure.

pub(crate) mod checks;
pub(crate) mod filename;
mod model;

pub(crate) use model::{AuditContext, AuditStatus, IssueType, Resolution, TagSnapshot};
