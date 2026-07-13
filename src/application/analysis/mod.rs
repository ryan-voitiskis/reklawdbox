//! Shared audio-analysis workflow.
//!
//! Both transports depend inward through this module:
//! `CLI/MCP -> application::analysis -> adapters + domain`.

pub(crate) mod batch;
pub(crate) mod identity;
pub(crate) mod job;
pub(crate) mod model;
