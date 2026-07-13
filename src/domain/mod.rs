//! Stable business concepts shared across Reklawdbox application layers.
//!
//! Domain modules own records and policy only. They must not depend on CLI,
//! MCP, database, store, XML, or tag-adapter infrastructure.

pub(crate) mod audit;
pub(crate) mod classification;
pub(crate) mod library;
pub(crate) mod metadata;
