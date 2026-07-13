//! Compatibility facade for writable local-state persistence.
//!
//! Canonical implementations live under [`crate::adapters::state`]. New
//! persistence code should be added there instead of extending this module.

#![allow(unused_imports)]

pub(crate) use crate::adapters::state::{
    AudioAnalysisIdentity, AuditFile, AuditIssue, AuditIssueRow, AuditSummary,
    BrokerDiscogsSession, CachedAudioAnalysis, ClearCachesResult, EnrichmentCacheEntry,
    EnrichmentKey, TimbralNormStats, WeightPresetEntry, batch_enrichment_existence,
    batch_enrichment_with_label, batch_enrichment_with_results,
    batch_fresh_audio_analysis_existence, batch_get_audio_analysis, batch_get_enrichment,
    batch_get_fresh_audio_analysis, clear_broker_discogs_session, clear_caches,
    clear_timbral_norm_stats, default_path, delete_missing_audit_files, delete_weight_preset,
    get_audio_analysis, get_audit_files_in_scope, get_audit_issues, get_audit_summary,
    get_broker_discogs_session, get_enrichment, get_open_issues_by_types, get_timbral_norm_stats,
    get_weight_preset, is_audio_analysis_fresh, list_weight_presets, mark_issues_resolved_for_path,
    open, open_read_only, resolve_audit_issues, resolve_path, save_timbral_norm_stats,
    save_weight_preset, set_audio_analysis_with_fingerprint, set_broker_discogs_session,
    set_enrichment, update_audit_issue_detail, upsert_audit_file, upsert_audit_issue,
};

#[cfg(test)]
pub(crate) use crate::adapters::state::{
    STORE_SCHEMA_VERSION, delete_audit_file, get_audit_file, get_audit_issue_by_id, migrate,
    resolve_path_from, set_audio_analysis, table_has_column,
};
