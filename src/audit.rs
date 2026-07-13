//! Compatibility facade for audit application workflows.
//!
//! Remove after Plan 046 once callers use application and domain paths directly.

#![allow(unused_imports)]

pub(crate) use crate::application::audit::{get_summary, query_issues, resolve_issues, scan};
pub(crate) use crate::domain::audit::IssueType;
