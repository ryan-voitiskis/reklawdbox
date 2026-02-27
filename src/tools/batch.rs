/// Shared progress counters and failure tracking for batch operations.
pub(super) struct BatchProgress {
    pub(super) processed: usize,
    pub(super) cached: usize,
    pub(super) skipped: usize,
    pub(super) failures: Vec<serde_json::Value>,
}

impl BatchProgress {
    pub(super) fn new() -> Self {
        Self {
            processed: 0,
            cached: 0,
            skipped: 0,
            failures: Vec::new(),
        }
    }
}
