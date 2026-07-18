use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Mutex, OnceLock, Weak};

use rusqlite::Connection;

use crate::adapters::providers::discogs;
use crate::domain::metadata::ChangeManager;

#[cfg(test)]
use super::enrichment::discogs_auth::DiscogsAuthTestDependencies;

pub(super) struct DatabaseContext {
    pub(super) db: OnceLock<Mutex<Connection>>,
    pub(super) effective_db_path: OnceLock<PathBuf>,
    pub(super) internal_db: OnceLock<Mutex<Connection>>,
    pub(super) db_path: Option<String>,
    /// Explicit store path override for tests and independently opened batch connections.
    pub(super) store_path: Option<String>,
}

pub(super) struct AnalysisContext {
    pub(super) essentia_python: OnceLock<Option<String>>,
    pub(super) essentia_python_override: Mutex<Option<String>>,
    pub(super) essentia_probe_invalidated: AtomicBool,
    pub(super) essentia_setup_lock: tokio::sync::Mutex<()>,
}

pub(super) struct EnrichmentContext {
    pub(super) http: reqwest::Client,
    pub(super) discogs_auth_lock: tokio::sync::Mutex<()>,
    pub(super) discogs_pending: Mutex<Option<discogs::PendingDeviceSession>>,
    #[cfg(test)]
    pub(super) discogs_auth_dependencies: Mutex<Option<DiscogsAuthTestDependencies>>,
}

pub(super) struct MutationContext {
    pub(super) xml_export_lock: tokio::sync::Mutex<()>,
    pub(super) audio_file_mutation_locks: Mutex<HashMap<PathBuf, Weak<tokio::sync::Mutex<()>>>>,
    pub(super) changes: ChangeManager,
    /// Number of unlabeled tracks left by backfill_labels; write_xml enforces this gate.
    pub(super) label_research_gate: std::sync::atomic::AtomicU32,
}

pub(super) struct ServerContext {
    pub(super) database: DatabaseContext,
    pub(super) analysis: AnalysisContext,
    pub(super) enrichment: EnrichmentContext,
    pub(super) mutation: MutationContext,
}

impl ServerContext {
    pub(super) fn new(db_path: Option<String>, http: reqwest::Client) -> Self {
        Self {
            database: DatabaseContext {
                db: OnceLock::new(),
                effective_db_path: OnceLock::new(),
                internal_db: OnceLock::new(),
                db_path,
                store_path: None,
            },
            analysis: AnalysisContext {
                essentia_python: OnceLock::new(),
                essentia_python_override: Mutex::new(None),
                essentia_probe_invalidated: AtomicBool::new(false),
                essentia_setup_lock: tokio::sync::Mutex::new(()),
            },
            enrichment: EnrichmentContext {
                http,
                discogs_auth_lock: tokio::sync::Mutex::new(()),
                discogs_pending: Mutex::new(None),
                #[cfg(test)]
                discogs_auth_dependencies: Mutex::new(None),
            },
            mutation: MutationContext {
                xml_export_lock: tokio::sync::Mutex::new(()),
                audio_file_mutation_locks: Mutex::new(HashMap::new()),
                changes: ChangeManager::new(),
                label_research_gate: std::sync::atomic::AtomicU32::new(0),
            },
        }
    }
}
