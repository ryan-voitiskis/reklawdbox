//! Metadata MCP tests grouped by capability.

mod backfill;
mod contracts;
mod export_backup;
mod support;
mod updates;

use crate::mcp::enrichment::{
    set_test_bandcamp_lookup_override, set_test_musicbrainz_lookup_override,
};
use crate::mcp::metadata::{
    BackfillAlbumsParams, BackfillLabelsParams, BackfillYearsParams, TrackChangeInput,
    UpdateTracksParams, WriteXmlParams, WriteXmlPlaylistInput, handle_update_tracks,
};
use crate::mcp::server::ReklawdboxServer;
use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use rmcp::ErrorData as McpError;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;

use crate::adapters::{rekordbox as db, state as store};
use crate::domain::classification::taxonomy as genre;
use crate::domain::metadata::ChangeManager;
use crate::domain::metadata::TrackChange;

use super::common::{
    call_tool_via_router, create_enrich_cache_writer_test_server,
    create_selector_pagination_test_db, create_server_with_connections,
    create_server_with_store_path, create_single_track_test_db, default_http_client_for_tests,
    extract_json, insert_test_track, make_test_track,
};
