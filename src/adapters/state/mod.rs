//! Writable local SQLite state owned by Reklawdbox.
//!
//! This is the only writable SQLite adapter boundary. Read-only state
//! connections never run migrations.

pub(crate) mod analysis;
mod audit;
mod broker;
pub(crate) mod classification;
mod connection;
mod enrichment;
mod maintenance;
mod migrations;
mod presets;

#[cfg(test)]
pub(crate) use analysis::{
    AudioAnalysisIdentity, TimbralNormStats, batch_get_audio_analysis, get_audio_analysis,
    get_timbral_norm_stats, is_audio_analysis_fresh, save_timbral_norm_stats, set_audio_analysis,
};
pub(crate) use analysis::{
    CachedAudioAnalysis, batch_fresh_audio_analysis_existence, batch_get_fresh_audio_analysis,
    set_audio_analysis_with_fingerprint,
};
pub(crate) use audit::{
    AuditFile, AuditIssue, AuditSummary, delete_missing_audit_files, get_audit_files_in_scope,
    get_audit_issues, get_audit_summary, get_open_issues_by_types, mark_issues_resolved_for_path,
    resolve_audit_issues, update_audit_issue_detail, upsert_audit_file, upsert_audit_issue,
};
#[cfg(test)]
pub(crate) use audit::{delete_audit_file, get_audit_file, get_audit_issue_by_id};
pub(crate) use broker::{
    clear_broker_discogs_session, get_broker_discogs_session, set_broker_discogs_session,
};
#[cfg(test)]
pub(crate) use connection::resolve_path_from;
pub(crate) use connection::{open, open_read_only, resolve_path};
pub(crate) use enrichment::{
    EnrichmentCacheEntry, batch_enrichment_existence, batch_enrichment_with_label,
    batch_enrichment_with_results, batch_get_enrichment, get_enrichment, set_enrichment,
};
pub(crate) use maintenance::clear_caches;
#[cfg(test)]
pub(crate) use migrations::{STORE_SCHEMA_VERSION, migrate, table_has_column};
pub(crate) use presets::{
    delete_weight_preset, get_weight_preset, list_weight_presets, save_weight_preset,
};

#[cfg(test)]
mod tests;
