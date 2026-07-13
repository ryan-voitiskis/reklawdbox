//! Discogs broker, authentication protocol, and lookup adapter.

#![allow(unused_imports)]

mod auth;
mod broker;
mod client;

pub(crate) use auth::*;
pub(crate) use broker::*;
pub(crate) use client::*;

super::rate_limit::define_rate_limiter!("REKLAWDBOX_DISCOGS_MIN_INTERVAL_MS", 1100);
