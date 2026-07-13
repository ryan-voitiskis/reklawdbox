//! Compatibility facade for provider rate limiting.

#![allow(unused_imports)]

pub(crate) use crate::adapters::providers::rate_limit::define_rate_limiter;
pub(crate) use crate::adapters::providers::rate_limit::{extract_retry_after, wait};
