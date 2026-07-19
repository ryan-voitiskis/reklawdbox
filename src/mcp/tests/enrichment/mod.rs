//! Enrichment MCP tests grouped by capability.

mod auth;
mod cache;
mod contracts;
mod resolve;
mod support;

use crate::mcp::enrichment::{
    BatchPage, DiscogsAuthTestDependencies, EnrichTracksParams, InMemoryDiscogsSessionPersistence,
    LookupDiscogsParams, ResolveFormat, ResolveTrackDataParams, ResolveTracksDataParams,
    auth_remediation_message, lookup_discogs_remote, lookup_output_with_cache_metadata,
    resolve_discogs_auth_transition_for_test, resolve_pending_tracks, resolve_single_track,
    set_test_discogs_lookup_override,
};
use crate::mcp::library::SearchFilterParams;
use crate::mcp::server::ReklawdboxServer;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rusqlite::{Connection, params};

use crate::adapters::state as store;
use crate::domain::metadata::{EditableField, TrackChange};

use super::common::{
    create_enrich_cache_writer_test_server, create_real_server_with_temp_store,
    create_selector_pagination_test_db, create_server_with_connections,
    create_server_with_store_path, create_single_track_test_db, default_http_client_for_tests,
    extract_json, insert_test_track, make_test_track, sample_real_tracks, set_test_audio_analysis,
    track_ids, write_test_audio_file,
};
