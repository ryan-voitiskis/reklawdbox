use super::transport::{ListCuesParams, PreviewCueRemovalParams, VerifyCueRemovalParams};
use crate::adapters::rekordbox::cues;
use crate::application::cues as removal;
use crate::mcp::{db_error, ok_json};
use rmcp::{ErrorData as McpError, model::CallToolResult};
use rusqlite::Connection;

pub(in crate::mcp) fn list(
    conn: &Connection,
    p: ListCuesParams,
) -> Result<CallToolResult, McpError> {
    ok_json(
        &cues::list_cues(
            conn,
            p.track_id.as_deref(),
            p.comment_exact.as_deref(),
            p.memory_points_only.unwrap_or(false),
            p.limit.unwrap_or(50).min(200),
            p.offset.unwrap_or(0),
        )
        .map_err(db_error)?,
    )
}

fn inventory(
    conn: &Connection,
    track: Option<&str>,
    comment: &str,
) -> Result<cues::RemovalInventory, McpError> {
    if comment.trim().is_empty() {
        return Err(McpError::invalid_params(
            "Provide a nonblank exact memory point comment",
            None,
        ));
    }
    cues::removal_inventory(conn, track, comment).map_err(db_error)
}

pub(in crate::mcp) fn preview(
    conn: &Connection,
    p: PreviewCueRemovalParams,
) -> Result<CallToolResult, McpError> {
    ok_json(&removal::preview(inventory(
        conn,
        p.track_id.as_deref(),
        &p.comment_exact,
    )?))
}

pub(in crate::mcp) fn verify(
    conn: &Connection,
    p: VerifyCueRemovalParams,
) -> Result<CallToolResult, McpError> {
    if p.verification_fingerprint.len() != 64
        || !p
            .verification_fingerprint
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(McpError::invalid_params(
            "Expected the lowercase SHA-256 fingerprint from preview_cue_removal",
            None,
        ));
    }
    ok_json(&removal::verify(
        inventory(conn, p.track_id.as_deref(), &p.comment_exact)?,
        &p.verification_fingerprint,
    ))
}
