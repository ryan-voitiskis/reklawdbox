//! Command-line surface: parsing, runtime policy, progress, and presentation.

mod analyze;
mod backup;
pub(crate) mod command;
mod hydrate;
mod mcp_config;
mod presentation;
pub(crate) mod runtime;
mod setup;
mod tags;

pub(crate) use command::run;
