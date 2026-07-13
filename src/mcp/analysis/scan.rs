//! MCP mapping for the transport-neutral audio filesystem adapter.

use rmcp::ErrorData as McpError;

pub(in crate::mcp) use crate::adapters::audio::scan_audio_directory;

pub(in crate::mcp) fn resolve_file_path(raw_path: &str) -> Result<String, McpError> {
    crate::adapters::audio::resolve_audio_path(raw_path)
        .map_err(|error| McpError::internal_error(error.to_string(), None))
}
