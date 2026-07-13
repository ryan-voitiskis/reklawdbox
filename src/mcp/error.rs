use rmcp::ErrorData as McpError;
use rmcp::model::{CallToolResult, Content};

pub(super) fn mcp_internal_error(msg: impl Into<String>) -> McpError {
    McpError::internal_error(msg.into(), None)
}

pub(super) fn ok_json(value: &impl serde::Serialize) -> Result<CallToolResult, McpError> {
    let json = serde_json::to_string(value).map_err(|e| mcp_internal_error(e.to_string()))?;
    Ok(CallToolResult::success(vec![Content::text(json)]))
}

pub(super) fn ok_structured_json<T>(value: T) -> Result<CallToolResult, McpError>
where
    T: serde::Serialize + schemars::JsonSchema,
{
    let value =
        serde_json::to_value(value).map_err(|error| mcp_internal_error(error.to_string()))?;
    Ok(CallToolResult::structured(value))
}

pub(super) fn db_error(error: rusqlite::Error) -> McpError {
    McpError::internal_error(format!("DB error: {error}"), None)
}

pub(super) fn cache_error(error: rusqlite::Error) -> McpError {
    McpError::internal_error(format!("Cache error: {error}"), None)
}
