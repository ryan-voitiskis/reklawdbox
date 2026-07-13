pub(in crate::mcp) struct BatchProgress {
    pub(in crate::mcp) processed: usize,
    pub(in crate::mcp) cached: usize,
    pub(in crate::mcp) skipped: usize,
    pub(in crate::mcp) failures: Vec<serde_json::Value>,
}

impl BatchProgress {
    pub(in crate::mcp) fn new() -> Self {
        Self {
            processed: 0,
            cached: 0,
            skipped: 0,
            failures: Vec::new(),
        }
    }
}
