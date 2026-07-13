//! Compatibility imports for analysis application helpers.

#![allow(unused_imports)]

pub(in crate::mcp) use crate::application::analysis::identity::{
    AudioCacheIdentity, audio_cache_identities_with_current_stratum_input, audio_cache_identity,
    check_analysis_cache, check_analysis_cache_for_identity, get_fresh_analysis_entry,
    get_fresh_analysis_entry_for_identity, resolved_audio_cache_key,
};

#[cfg(test)]
pub(in crate::mcp) use crate::application::analysis::identity::{
    audio_cache_identities_with_fingerprint_loader,
    audio_cache_identity_with_stratum_input_fingerprint,
};
